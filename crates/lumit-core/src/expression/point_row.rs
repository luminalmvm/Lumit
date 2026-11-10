//! Finding the point a row is one number of, on a layer.

use super::Row;

/// The properties of the point `row` is one number of on `layer`, in order,
/// and which of them it is.
pub(super) fn point_of_row<'l>(
    layer: &'l crate::model::Layer,
    row: Row<'_>,
) -> Option<(Vec<&'l crate::anim::Property>, usize)> {
    use crate::model::EffectValue;
    match row {
        Row::Transform(prop) => {
            let (rows, axis) = layer.point_rows(prop);
            let props = rows
                .iter()
                .map(|r| layer.prop(*r))
                .collect::<Option<Vec<_>>>()?;
            Some((props, axis))
        }
        Row::Effect { effect, param } => {
            let inst = layer.effects.iter().find(|e| e.id == effect)?;
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
    }
}
