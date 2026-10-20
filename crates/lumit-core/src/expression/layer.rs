use std::sync::Arc;

use rhai::plugin::*;
use uuid::Uuid;

use crate::expression::space::Way;
use crate::{expression::ExpressionContext, model};

#[derive(Clone, CustomType)]
pub struct Layer {
    /// `(comp id, layer id)` when the reference resolved; `None` is the
    /// invalid-reference sentinel every accessor answers politely for.
    ids: Option<(Uuid, Uuid)>,
}

/// Find the referenced layer and hand `read` a borrow of it, along with the
/// call's context. The accessors below want one field or one property each,
/// and a layer read happens per property per frame — so nothing here clones
/// the layer (its keyframes, effects, masks and paint included) just to look
/// at it.
fn with_layer<T>(
    context: &NativeCallContext,
    this: &Layer,
    read: impl FnOnce(&Arc<ExpressionContext>, &model::Layer) -> T,
) -> Option<T> {
    let (comp_id, layer_id) = this.ids?;
    let context = ExpressionContext::from_call(context);
    let comp = context.document.comp(comp_id)?;
    let layer = comp.layers.iter().find(|l| l.id == layer_id)?;
    Some(read(&context, layer))
}

fn _time(context: &NativeCallContext, this: &mut Layer) -> f64 {
    with_layer(context, this, |context, layer| {
        context.comp_time - layer.in_point.0.to_f64()
    })
    .unwrap_or(-1.0)
}

/// One of the referenced layer's transform properties, evaluated at that
/// layer's own time.
///
/// Every property getter below is this same handful of steps with a different
/// field picked out, so they share it: find the layer, work out its local time,
/// and evaluate the property under a context that has been re-pointed at *that*
/// layer and stepped one level deeper — the depth is what stops two properties
/// that refer to each other from recursing forever.
fn transform_property(
    call: &NativeCallContext,
    this: &mut Layer,
    pick: impl Fn(&model::TransformGroup) -> &crate::anim::Property,
) -> f64 {
    layer_property(call, this, |layer| Some(pick(&layer.transform)))
}

/// [`transform_property`] for any of the layer's properties, a camera's zoom
/// among them. A layer that has no such property answers the miss value.
fn layer_property(
    call: &NativeCallContext,
    this: &mut Layer,
    pick: impl Fn(&model::Layer) -> Option<&crate::anim::Property>,
) -> f64 {
    with_layer(call, this, |context, layer| {
        let t = crate::time::layer_time(context.comp_time, layer.start_offset.0);
        let mut deeper = context.increase_depth();
        deeper.layer = Some(layer.id);
        Some(pick(layer)?.value_at_with_context(t, Arc::new(deeper)))
    })
    .flatten()
    .unwrap_or(-1.0)
}

/// Up to three numbers out of a list, the missing ones nought.
fn point_of(list: &rhai::Array) -> (f64, f64, f64) {
    let at = |i: usize| {
        list.get(i)
            .cloned()
            .and_then(crate::expression::as_f64)
            .unwrap_or(0.0)
    };
    (at(0), at(1), at(2))
}

fn list_of(p: (f64, f64, f64), len: usize) -> rhai::Array {
    [p.0, p.1, p.2]
        .into_iter()
        .take(len)
        .map(Dynamic::from_float)
        .collect()
}

/// `point` carried `way` between the referenced layer's spaces at comp time
/// `t`, or now. An empty list when there is no answer, or the expression has
/// spent its budget.
fn convert(
    call: &NativeCallContext,
    this: &Layer,
    point: &rhai::Array,
    t: Option<Dynamic>,
    way: Way,
) -> rhai::Array {
    let found = || {
        let (comp_id, layer_id) = this.ids?;
        let context = ExpressionContext::from_call(call);
        let comp = context.document.comp(comp_id)?;
        let layer = comp.layers.iter().find(|l| l.id == layer_id)?;
        let t = t
            .and_then(crate::expression::as_f64)
            .filter(|t| t.is_finite())
            .unwrap_or(context.comp_time);
        let read = super::space::reader(&context, comp_id, t);
        let space = super::space::space(&context.document, comp, layer, t, &read);
        // Each layer placed reads nine properties.
        if !crate::expression::script::charge(space.cost.saturating_mul(9)) {
            return None;
        }
        let out = space.carry(way, point_of(point), point.len())?;
        Some(out.into_iter().map(Dynamic::from_float).collect())
    };
    found().unwrap_or_default()
}

impl Layer {
    /// A reference to a layer by its comp's id and its own.
    pub(crate) fn of(ids: Option<(Uuid, Uuid)>) -> Layer {
        Layer { ids }
    }
}

// Rhai's `#[export_module]` expands to argument-unwrapping code of its own,
// which trips `clippy::unwrap_used` on the generated `&mut` receivers. The
// lint is about *our* unwraps, and there is no way to spell these differently
// short of dropping the macro, so it is switched off for the generated module
// only — not for the module's callers, and not for the helpers above.
#[allow(clippy::unwrap_used)]
#[export_module]
pub mod layers {
    use Layer;

    /// get the current layer
    pub fn layer(context: NativeCallContext) -> Layer {
        let context = ExpressionContext::from_call(&context);
        Layer {
            ids: context.comp.zip(context.layer),
        }
    }

    #[rhai_fn(name = "layer")]
    /// get a layer by name
    pub fn layer_by_name(context: NativeCallContext, name: String) -> Layer {
        let context = ExpressionContext::from_call(&context);
        Layer {
            ids: context.comp.and_then(|c| {
                let comp = context.document.comp(c)?;
                let layer = comp.layers.iter().find(|f| f.name == name)?;
                Some((comp.id, layer.id))
            }),
        }
    }

    /// get the name of a layer
    #[rhai_fn(get = "name")]
    pub fn name(context: NativeCallContext, this: &mut Layer) -> String {
        with_layer(&context, this, |_, layer| layer.name.clone())
            .unwrap_or_else(|| "Invalid Layer Reference".into())
    }

    /// get the current time of this layer
    #[rhai_fn(get = "time")]
    pub fn time(context: NativeCallContext, this: &mut Layer) -> f64 {
        _time(&context, this)
    }

    /// x coordinate of the layer's position
    #[rhai_fn(get = "x")]
    pub fn x(context: NativeCallContext, this: &mut Layer) -> f64 {
        transform_property(&context, this, |tr| &tr.position_x)
    }

    /// y coordinate of the layer's position
    #[rhai_fn(get = "y")]
    pub fn y(context: NativeCallContext, this: &mut Layer) -> f64 {
        transform_property(&context, this, |tr| &tr.position_y)
    }

    /// value of the layer's rotation
    #[rhai_fn(get = "rotation")]
    pub fn rotation(context: NativeCallContext, this: &mut Layer) -> f64 {
        transform_property(&context, this, |tr| &tr.rotation)
    }

    /// x component of the layer's scale
    #[rhai_fn(get = "scale_x")]
    pub fn scale_x(context: NativeCallContext, this: &mut Layer) -> f64 {
        transform_property(&context, this, |tr| &tr.scale_x)
    }

    /// y component of the layer's scale
    #[rhai_fn(get = "scale_y")]
    pub fn scale_y(context: NativeCallContext, this: &mut Layer) -> f64 {
        transform_property(&context, this, |tr| &tr.scale_y)
    }

    /// x coordinate of the layer's anchor
    #[rhai_fn(get = "anchor_x")]
    pub fn anchor_x(context: NativeCallContext, this: &mut Layer) -> f64 {
        transform_property(&context, this, |tr| &tr.anchor_x)
    }

    /// y coordinate of the layer's anchor
    #[rhai_fn(get = "anchor_y")]
    pub fn anchor_y(context: NativeCallContext, this: &mut Layer) -> f64 {
        transform_property(&context, this, |tr| &tr.anchor_y)
    }

    /// layer's current opacity
    #[rhai_fn(get = "opacity")]
    pub fn opacity(context: NativeCallContext, this: &mut Layer) -> f64 {
        transform_property(&context, this, |tr| &tr.opacity)
    }

    /// z coordinate of the layer's position
    #[rhai_fn(get = "z")]
    pub fn z(context: NativeCallContext, this: &mut Layer) -> f64 {
        transform_property(&context, this, |tr| &tr.position_z)
    }

    /// the layer's X Rotation
    #[rhai_fn(get = "rotation_x")]
    pub fn rotation_x(context: NativeCallContext, this: &mut Layer) -> f64 {
        transform_property(&context, this, |tr| &tr.rotation_x)
    }

    /// the layer's Y Rotation
    #[rhai_fn(get = "rotation_y")]
    pub fn rotation_y(context: NativeCallContext, this: &mut Layer) -> f64 {
        transform_property(&context, this, |tr| &tr.rotation_y)
    }

    /// the layer's Z Rotation, the same as rotation
    #[rhai_fn(get = "rotation_z")]
    pub fn rotation_z(context: NativeCallContext, this: &mut Layer) -> f64 {
        transform_property(&context, this, |tr| &tr.rotation)
    }

    /// the layer's Orientation, [x, y, z], as it turns between its keys
    #[rhai_fn(get = "orientation")]
    pub fn orientation(context: NativeCallContext, this: &mut Layer) -> rhai::Array {
        let o = with_layer(&context, this, |context, layer| {
            let t = crate::time::layer_time(context.comp_time, layer.start_offset.0);
            let mut deeper = context.increase_depth();
            deeper.layer = Some(layer.id);
            let deeper = Arc::new(deeper);
            let read = |p: &crate::anim::Property| p.value_at_with_context(t, deeper.clone());
            layer.transform.orientation_at(t, &read)
        });
        super::list_of(o.unwrap_or((-1.0, -1.0, -1.0)), 3)
    }

    /// z coordinate of the layer's anchor
    #[rhai_fn(get = "anchor_z")]
    pub fn anchor_z(context: NativeCallContext, this: &mut Layer) -> f64 {
        transform_property(&context, this, |tr| &tr.anchor_z)
    }

    /// z component of the layer's scale
    #[rhai_fn(get = "scale_z")]
    pub fn scale_z(context: NativeCallContext, this: &mut Layer) -> f64 {
        transform_property(&context, this, |tr| &tr.scale_z)
    }

    /// a camera's zoom
    #[rhai_fn(get = "zoom")]
    pub fn zoom(context: NativeCallContext, this: &mut Layer) -> f64 {
        layer_property(&context, this, |layer| match &layer.kind {
            model::LayerKind::Camera { zoom, .. } => Some(zoom),
            _ => None,
        })
    }

    /// a camera's point of interest, [x, y, z]
    #[rhai_fn(get = "point_of_interest")]
    pub fn point_of_interest(context: NativeCallContext, this: &mut Layer) -> rhai::Array {
        let mut axis = |i: usize| {
            layer_property(&context, this, |layer| match &layer.kind {
                model::LayerKind::Camera { options, .. } => options.point_of_interest.get(i),
                _ => None,
            })
        };
        let p = (axis(0), axis(1), axis(2));
        super::list_of(p, 3)
    }

    /// a point in the layer's pixels to where it is on the picture
    pub fn to_comp(
        context: NativeCallContext,
        this: &mut Layer,
        point: rhai::Array,
    ) -> rhai::Array {
        super::convert(&context, this, &point, None, Way::ToComp)
    }

    /// a point in the layer's pixels to where it is on the picture, at comp time `t`
    #[rhai_fn(name = "to_comp")]
    pub fn to_comp_at(
        context: NativeCallContext,
        this: &mut Layer,
        point: rhai::Array,
        t: Dynamic,
    ) -> rhai::Array {
        super::convert(&context, this, &point, Some(t), Way::ToComp)
    }

    /// a point on the picture to the layer's pixels
    pub fn from_comp(
        context: NativeCallContext,
        this: &mut Layer,
        point: rhai::Array,
    ) -> rhai::Array {
        super::convert(&context, this, &point, None, Way::FromComp)
    }

    /// a point on the picture to the layer's pixels, at comp time `t`
    #[rhai_fn(name = "from_comp")]
    pub fn from_comp_at(
        context: NativeCallContext,
        this: &mut Layer,
        point: rhai::Array,
        t: Dynamic,
    ) -> rhai::Array {
        super::convert(&context, this, &point, Some(t), Way::FromComp)
    }

    /// a point in the layer's pixels to the comp's 3D space
    pub fn to_world(
        context: NativeCallContext,
        this: &mut Layer,
        point: rhai::Array,
    ) -> rhai::Array {
        super::convert(&context, this, &point, None, Way::ToWorld)
    }

    /// a point in the layer's pixels to the comp's 3D space, at comp time `t`
    #[rhai_fn(name = "to_world")]
    pub fn to_world_at(
        context: NativeCallContext,
        this: &mut Layer,
        point: rhai::Array,
        t: Dynamic,
    ) -> rhai::Array {
        super::convert(&context, this, &point, Some(t), Way::ToWorld)
    }

    /// a point in the comp's 3D space to the layer's pixels
    pub fn from_world(
        context: NativeCallContext,
        this: &mut Layer,
        point: rhai::Array,
    ) -> rhai::Array {
        super::convert(&context, this, &point, None, Way::FromWorld)
    }

    /// a point in the comp's 3D space to the layer's pixels, at comp time `t`
    #[rhai_fn(name = "from_world")]
    pub fn from_world_at(
        context: NativeCallContext,
        this: &mut Layer,
        point: rhai::Array,
        t: Dynamic,
    ) -> rhai::Array {
        super::convert(&context, this, &point, Some(t), Way::FromWorld)
    }

    /// a direction on the layer to one on the picture
    pub fn to_comp_vec(
        context: NativeCallContext,
        this: &mut Layer,
        point: rhai::Array,
    ) -> rhai::Array {
        super::convert(&context, this, &point, None, Way::ToCompVec)
    }

    /// a direction on the layer to one on the picture, at comp time `t`
    #[rhai_fn(name = "to_comp_vec")]
    pub fn to_comp_vec_at(
        context: NativeCallContext,
        this: &mut Layer,
        point: rhai::Array,
        t: Dynamic,
    ) -> rhai::Array {
        super::convert(&context, this, &point, Some(t), Way::ToCompVec)
    }

    /// a direction on the picture to one on the layer
    pub fn from_comp_vec(
        context: NativeCallContext,
        this: &mut Layer,
        point: rhai::Array,
    ) -> rhai::Array {
        super::convert(&context, this, &point, None, Way::FromCompVec)
    }

    /// a direction on the picture to one on the layer, at comp time `t`
    #[rhai_fn(name = "from_comp_vec")]
    pub fn from_comp_vec_at(
        context: NativeCallContext,
        this: &mut Layer,
        point: rhai::Array,
        t: Dynamic,
    ) -> rhai::Array {
        super::convert(&context, this, &point, Some(t), Way::FromCompVec)
    }

    /// a direction on the layer to one in the comp's 3D space
    pub fn to_world_vec(
        context: NativeCallContext,
        this: &mut Layer,
        point: rhai::Array,
    ) -> rhai::Array {
        super::convert(&context, this, &point, None, Way::ToWorldVec)
    }

    /// a direction on the layer to one in the comp's 3D space, at comp time `t`
    #[rhai_fn(name = "to_world_vec")]
    pub fn to_world_vec_at(
        context: NativeCallContext,
        this: &mut Layer,
        point: rhai::Array,
        t: Dynamic,
    ) -> rhai::Array {
        super::convert(&context, this, &point, Some(t), Way::ToWorldVec)
    }

    /// a direction in the comp's 3D space to one on the layer
    pub fn from_world_vec(
        context: NativeCallContext,
        this: &mut Layer,
        point: rhai::Array,
    ) -> rhai::Array {
        super::convert(&context, this, &point, None, Way::FromWorldVec)
    }

    /// a direction in the comp's 3D space to one on the layer, at comp time `t`
    #[rhai_fn(name = "from_world_vec")]
    pub fn from_world_vec_at(
        context: NativeCallContext,
        this: &mut Layer,
        point: rhai::Array,
        t: Dynamic,
    ) -> rhai::Array {
        super::convert(&context, this, &point, Some(t), Way::FromWorldVec)
    }

    /// a point on the picture to where it meets the layer's own plane, [x, y]
    pub fn from_comp_to_surface(
        context: NativeCallContext,
        this: &mut Layer,
        point: rhai::Array,
    ) -> rhai::Array {
        super::convert(&context, this, &point, None, Way::FromCompToSurface)
    }

    /// a point on the picture to where it meets the layer's own plane, [x, y], at comp time `t`
    #[rhai_fn(name = "from_comp_to_surface")]
    pub fn from_comp_to_surface_at(
        context: NativeCallContext,
        this: &mut Layer,
        point: rhai::Array,
        t: Dynamic,
    ) -> rhai::Array {
        super::convert(&context, this, &point, Some(t), Way::FromCompToSurface)
    }
}
