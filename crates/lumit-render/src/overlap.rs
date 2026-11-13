//! Two clips overlapping on a picture row, mixed where they are drawn: the
//! dissolve of one that plays a composition or a solid, and every Transition
//! effect (`lumit_core::sequence::transition`).
//!
//! A Transition kind runs on the outgoing clip, or on both clips, across the
//! overlap. So the two footage pictures can't be mixed where they are decoded,
//! as a plain dissolve's are. The outgoing one comes along beside the incoming
//! one instead, filed under a moment nothing else asks for.

use std::sync::Arc;

use lumit_core::expression::ExpressionContext;
use lumit_core::fx::markers::MarkerContext;
use lumit_core::model::{Composition, EffectInstance, Layer};
use lumit_core::sequence::transition::{self, Side, Sweep};
use lumit_core::sequence::Clip;
use lumit_gpu::composite::Blend;

use crate::build::read_draw;
use crate::decode::{CompJob, CompLayerPixels, Cut};
use crate::draw::{CompLayerDraw, DrawSource};

/// The moment an outgoing clip's picture is filed under on the incoming clip's
/// layer. No temporal effect asks for it, as none is ever this far away.
const ACROSS: f64 = f64::INFINITY;

/// What the incoming clip's job carries of the outgoing clip inside an overlap.
/// `whole` is a job for another moment, which a temporal effect above reads, and
/// that stays a dissolve.
pub(crate) fn under_incoming(
    incoming: &Clip,
    whole: bool,
    gain: f64,
    under: Option<CompJob>,
) -> Cut {
    if whole || incoming.drawn_transition().is_none() {
        Cut::Under(gain, under)
    } else {
        Cut::Moment(ACROSS, under)
    }
}

/// The outgoing clip's picture where it came along beside the incoming one.
pub(crate) fn across(lp: &CompLayerPixels) -> Option<&CompLayerPixels> {
    let moment = lp.shutter.iter().find(|(at, _)| *at == ACROSS);
    moment.and_then(|(_, picture)| picture.as_deref())
}

/// A clip's picture and its natural size.
type Picture = (DrawSource, (f32, f32));

/// One of the mix's two draws, fitted about the middle of the picture it is
/// mixed on, `natural`.
fn part(layer: &Layer, (source, size): Picture, natural: (f32, f32)) -> CompLayerDraw {
    let mut draw = read_draw(layer, source, size);
    draw.position = (natural.0 * 0.5, natural.1 * 0.5);
    let fit = (natural.0 / size.0.max(1.0)).min(natural.1 / size.1.max(1.0));
    draw.scale = (fit * 100.0, fit * 100.0);
    // Its stack is resolved at the picture's own size, and drawn at the
    // preview's, so px rows follow the picture down.
    draw.fx_ref_width = Some(size.0);
    draw
}

/// The two drawn as one picture the size of the incoming clip's, which is the
/// layer's across the overlap as it is where two footage clips are mixed. A mix
/// has no name of its own, and each composition inside it keeps the one it was
/// given.
fn mix(draws: Vec<CompLayerDraw>, natural: (f32, f32), lt: f64) -> Picture {
    (
        DrawSource::Nested {
            width: natural.0 as u32,
            height: natural.1 as u32,
            background: [0.0, 0.0, 0.0, 0.0],
            draws,
            camera: None,
            key: None,
            paint: Vec::new(),
            paint_time: lt,
            flow: None,
        },
        natural,
    )
}

/// A footage or solid picture drawn once into one of its own size, so its
/// colour has its alpha in it before a stack runs on it, as a comp clip's has.
fn whole(layer: &Layer, picture: Picture, lt: f64) -> Picture {
    match picture.0 {
        DrawSource::Pixels { .. } => {
            let size = picture.1;
            mix(vec![part(layer, picture, size)], size, lt)
        }
        _ => picture,
    }
}

/// A dissolve: both, each at its share, the second laid on the first with
/// [`Blend::Plus`]. That is the crossfade the decode worker makes of two footage
/// clips (`lumit_core::pixels::dissolve_rgba`), and two whole pictures sum to a
/// whole one.
pub(crate) fn dissolved(
    layer: &Layer,
    going: Picture,
    away: f64,
    coming: Picture,
    here: f64,
    lt: f64,
) -> Picture {
    let natural = coming.1;
    let mut under = part(layer, going, natural);
    under.opacity = (away * 100.0) as f32;
    let mut over = part(layer, coming, natural);
    over.opacity = (here * 100.0) as f32;
    over.blend = Blend::Plus;
    mix(vec![under, over], natural, lt)
}

/// A clip's own stack ready to run on its picture, on the layer's clock, as a
/// clip's keys are stored from its own start.
fn own_stack(clip: &Clip) -> Vec<EffectInstance> {
    if !clip.fx {
        return Vec::new();
    }
    let mut stack = clip.effects.clone();
    lumit_core::preset::shift_keys(&mut stack, clip.place_start);
    stack
}

/// A transition: the outgoing clip with the effect run on it after its own
/// stack, swept to `u`, over the incoming clip whole. A kind that sweeps both
/// clips runs it on the incoming clip too, and a Peak kind dissolves the two.
/// Spatial rows are px@comp, and each stack runs on its clip's own picture, as
/// a layer's does. The effect's keys are stored from the incoming clip's start.
#[allow(clippy::too_many_arguments)]
pub(crate) fn wiped(
    layer: &Layer,
    comp: &Composition,
    going: Picture,
    coming: Picture,
    effect: &EffectInstance,
    (outgoing, incoming): (&Clip, &Clip),
    u: f64,
    (lt, frame_lt): (f64, f64),
    context: Arc<ExpressionContext>,
) -> Picture {
    let (going, coming) = (whole(layer, going, lt), whole(layer, coming, lt));
    let natural = coming.1;
    let size = going.1;
    let local = lt - incoming.place_start.to_f64();
    let swept = |side, (w, h): (f32, f32)| {
        let mut swept = [transition::swept(
            effect,
            u,
            side,
            f64::from(w),
            f64::from(h),
            local,
        )];
        lumit_core::preset::shift_keys(&mut swept, incoming.place_start);
        swept
    };
    let mut gone = own_stack(outgoing);
    gone.extend(swept(Side::Out, size));
    let mut come = own_stack(incoming);
    let sweep = transition::kind(&effect.effect.match_name).map(|k| k.sweep);
    if matches!(sweep, Some(Sweep::Both | Sweep::Peak { .. })) {
        come.extend(swept(Side::In, natural));
    }
    let diag = ((comp.width as f32).powi(2) + (comp.height as f32).powi(2)).sqrt();
    let markers = MarkerContext::for_layer(comp, layer);
    // Both pictures are whole ones now, drawn at the comp's own scale.
    let run = |draw: &mut CompLayerDraw, stack: &[EffectInstance]| {
        let (fx_ids, fx) = lumit_core::fx::resolved::resolve_stack_temporal_named(
            stack,
            &Default::default(),
            lt,
            frame_lt,
            diag,
            1.0,
            &markers,
            context.clone(),
        );
        draw.fx = fx;
        draw.fx_ids = fx_ids;
    };
    let mut coming = part(layer, coming, natural);
    run(&mut coming, &come);
    let mut going = part(layer, going, natural);
    run(&mut going, &gone);
    let draws = match transition::share(effect, u) {
        // As `dissolved` mixes them.
        Some(here) => {
            going.opacity = ((1.0 - here) * 100.0) as f32;
            coming.opacity = (here * 100.0) as f32;
            coming.blend = Blend::Plus;
            vec![going, coming]
        }
        None if transition::incoming_over(effect) => vec![going, coming],
        None => vec![coming, going],
    };
    mix(draws, natural, lt)
}
