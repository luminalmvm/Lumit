//! Two clips overlapping on a picture row, mixed where they are drawn: the
//! dissolve of one that plays a composition or a solid, and every Transition
//! effect (`lumit_core::sequence::transition`).
//!
//! A transition runs its effect over the outgoing clip, with the incoming clip
//! beneath. So the two footage pictures can't be mixed where they are decoded,
//! as a plain dissolve's are. The outgoing one comes along beside the incoming
//! one instead, filed under a moment nothing else asks for.

use std::sync::Arc;

use lumit_core::expression::ExpressionContext;
use lumit_core::fx::markers::MarkerContext;
use lumit_core::model::{Composition, EffectInstance, Layer};
use lumit_core::sequence::{transition, Clip};
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

/// A transition: the incoming clip whole, and the outgoing clip over it with the
/// effect run on it after its own stack, swept to `u`. Spatial rows are px@comp,
/// and each stack runs on its clip's own picture, as a layer's does. The effect's
/// keys are stored from the incoming clip's start.
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
    let natural = coming.1;
    let size = going.1;
    let local = lt - incoming.place_start.to_f64();
    let mut swept = [transition::swept(
        effect,
        u,
        f64::from(size.0),
        f64::from(size.1),
        local,
    )];
    lumit_core::preset::shift_keys(&mut swept, incoming.place_start);
    let mut gone = own_stack(outgoing);
    gone.extend(swept);
    let diag = ((comp.width as f32).powi(2) + (comp.height as f32).powi(2)).sqrt();
    let markers = MarkerContext::for_layer(comp, layer);
    let run = |draw: &mut CompLayerDraw, stack: &[EffectInstance], picture_scale: f32| {
        let (fx_ids, fx) = lumit_core::fx::resolved::resolve_stack_temporal_named(
            stack,
            &Default::default(),
            lt,
            frame_lt,
            diag * picture_scale,
            picture_scale,
            &markers,
            context.clone(),
        );
        draw.fx = fx;
        draw.fx_ids = fx_ids;
    };
    let scale_of = |picture: &Picture| match &picture.0 {
        DrawSource::Pixels { tex_w, .. } => *tex_w as f32 / picture.1 .0.max(1.0),
        _ => 1.0,
    };
    let (going_scale, coming_scale) = (scale_of(&going), scale_of(&coming));
    let mut under = part(layer, coming, natural);
    run(&mut under, &own_stack(incoming), coming_scale);
    let mut over = part(layer, going, natural);
    run(&mut over, &gone, going_scale);
    mix(vec![under, over], natural, lt)
}
