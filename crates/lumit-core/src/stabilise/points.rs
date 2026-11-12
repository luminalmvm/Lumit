//! The followed points as the Viewer draws them: where the stabilised picture
//! shows each one, and whether it moved with the picture.

use std::sync::Arc;

use super::motion::{apply, invert, within};
use super::place::layer_to_comp;
use super::plan::judged_by;
use super::rows::Reader;
use super::shown::{nearest, shown};
use crate::fx::effects::stabilise::{planned_held, Held, Planned};
use crate::model::{Composition, Document, EffectInstance, Layer};

/// One followed point as the Viewer draws it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShownPoint {
    /// The tracker's id: what a pick names and a drop takes.
    pub track: u32,
    /// Comp pixels, where the stabilised picture shows it.
    pub at: [f64; 2],
    /// Whether it moved with the picture there, as the Method fits it. A
    /// point that did not was set aside by the fit.
    pub kept: bool,
}

/// The points `inst` followed on `layer`, showing at comp time `t`, where the
/// stabilised picture draws them, with the generation of the analysis they
/// belong to. Dropped points are left out, and none outside the analysed span.
/// Never makes a plan: with none in hand it is made on a worker, and this
/// answers [`Held::Making`].
#[must_use]
pub fn points_at(
    doc: &Arc<Document>,
    comp: &Composition,
    layer: &Layer,
    inst: &EffectInstance,
    t: f64,
) -> Held<(u64, Vec<ShownPoint>)> {
    let r = Reader {
        doc,
        comp,
        layer,
        inst,
    };
    planned_held(&r).map(|p| (p.filed.generation, placed(&r, &p, t).unwrap_or_default()))
}

/// The points of `p` showing at comp time `t`, placed: `None` outside the
/// analysed span.
fn placed(r: &Reader<'_>, p: &Planned, t: f64) -> Option<Vec<ShownPoint>> {
    let (doc, comp, layer) = (r.doc, r.comp, r.layer);
    let a = &p.filed.analysis;
    let lt = crate::time::layer_time(t, layer.start_offset.0);
    // The points of the frame the picture shows, as the decode picks it.
    if !(0.0..f64::from(a.frames)).contains(&nearest(layer, lt, p.rate, a)) {
        return None;
    }
    let picked = shown(layer, r.comp_fps(), lt, p.rate, a)?;
    let i = u32::try_from(picked.nearest()).ok()?;
    let fm = p.plan.at(picked)?;
    let (plan, rows) = (&p.plan, &p.rows);
    // Input pixel to output pixel: the frame's map, as drawn, run
    // backwards. The rolling shutter is left out, a few pixels at most.
    let shown = invert(&r.drawn(lt, &fm, p.size()))?;
    let place = layer_to_comp(doc, comp, layer, t)?;
    // Whether a point moved with the picture: the step out of this frame,
    // or into it on the last one.
    let (from, to) = if i + 1 < a.frames {
        (i, i + 1)
    } else {
        (i.saturating_sub(1), i)
    };
    let judge = plan
        .steps
        .get(from as usize)
        .and_then(|s| judged_by(s, rows.model));
    let agree = a.agree_px();
    let out = a
        .runs
        .iter()
        .filter(|r| rows.dropped.binary_search(&r.id).is_err())
        .filter_map(|r| {
            let p = r.at(i)?;
            let p = [f64::from(p[0]), f64::from(p[1])];
            let kept = match (&judge, r.at(from), r.at(to)) {
                (Some(m), Some(u), Some(v)) => within(
                    m,
                    &(
                        [f64::from(u[0]), f64::from(u[1])],
                        [f64::from(v[0]), f64::from(v[1])],
                    ),
                    agree,
                ),
                _ => false,
            };
            let o = apply(&shown, p)?;
            let c = apply(&place, o)?;
            Some(ShownPoint {
                track: r.id,
                at: c,
                kept,
            })
        })
        .collect();
    Some(out)
}
