use super::Row;

/// The properties of the point `row` is one number of on `layer`, in order,
/// and which of them it is. An effect is found wherever it sits in `comp`.
pub(super) fn point_of_row<'l>(
    comp: &'l crate::model::Composition,
    layer: &'l crate::model::Layer,
    row: Row<'_>,
) -> Option<(Vec<&'l crate::anim::Property>, usize)> {
    match row {
        Row::Transform(prop) => {
            let (rows, axis) = layer.point_rows(prop);
            let props = rows
                .iter()
                .map(|r| layer.prop(*r))
                .collect::<Option<Vec<_>>>()?;
            Some((props, axis))
        }
        Row::Effect { effect, param } => effect_row(comp, effect, param, 0),
        Row::Channel {
            effect,
            param,
            channel,
        } => effect_row(comp, effect, param, channel),
    }
}

fn effect_row<'l>(
    comp: &'l crate::model::Composition,
    effect: uuid::Uuid,
    param: &str,
    channel: usize,
) -> Option<(Vec<&'l crate::anim::Property>, usize)> {
    use crate::model::EffectValue;
    let inst = comp.effects().find(|e| e.id == effect)?;
    if let EffectValue::Colour(channels) = &inst.params.iter().find(|p| p.id == param)?.value {
        return Some((channels.iter().collect(), channel.min(3)));
    }
    let float = |id: &str| match inst.params.iter().find(|p| p.id == id)?.value {
        EffectValue::Float(ref p) => Some(p),
        _ => None,
    };
    let pair = crate::fx::builtins::schema(&inst.effect.match_name)
        .and_then(|schema| schema.pairs().find(|p| p.x == param || p.y == param));
    if let Some(pair) = pair {
        if let (Some(x), Some(y)) = (float(pair.x), float(pair.y)) {
            return Some((vec![x, y], usize::from(pair.y == param)));
        }
    }
    Some((vec![float(param)?], 0))
}
