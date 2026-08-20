//! Properties and keyframes: the value copy (K-025).
//!
//! # In plain terms
//!
//! Lumit's keyframes are deliberately the same mathematics as After Effects'
//! (a cubic described by a *speed* and an *influence* on each side of every
//! key), so nothing here re-computes a curve — it copies one. What this module
//! actually does is unpick the four ways After Effects can present the same
//! animation:
//!
//! - a plain still value;
//! - a list of keyframes, where a two- or three-dimensional property packs all
//!   its axes into one key and Lumit keeps an axis per lane;
//! - a *separated* property, where the animation is not on the property at all
//!   but on one follower per axis — read the property itself and you import a
//!   moving layer as a frozen one;
//! - an expression, which drives the property instead of its keys.
//!
//! Two of AE's per-key flags are deliberately *not* carried, and both for the
//! same reason: the capture already contains the answer they describe.
//! `roving` keys have had their time worked out by AE and recorded at the time
//! they ended up at, and `auto_bezier`/`continuous` keys have had their ease
//! worked out and recorded in `in_ease`/`out_ease`. Copying the resolved
//! numbers reproduces the curve exactly; copying the flag would ask Lumit to
//! re-derive something it has no need to.

use lumit_core::anim::{Animation, Keyframe, Property as LumProperty, SideInterp};
use lumit_core::time::Rational;

use crate::capture::{Ease, Keyframe as AeKey, Property};
use crate::report::{ItemPath, Outcome, Reason};

use super::Conv;

/// The direct child of `props` with this match name, group or leaf.
pub(crate) fn child<'a>(props: &'a [Property], match_name: &str) -> Option<&'a Property> {
    props
        .iter()
        .find(|p| p.match_name.as_deref() == Some(match_name))
}

/// The children of the direct child group with this match name (empty when
/// there is no such group, which is the ordinary case for a layer with no
/// masks or no effects).
pub(crate) fn group<'a>(props: &'a [Property], match_name: &str) -> &'a [Property] {
    child(props, match_name).map_or(&[], Property::children)
}

/// The nearest node with this match name: the direct child if there is one,
/// otherwise the first found anywhere below.
///
/// The fallback exists for the handful of properties After Effects keeps
/// inside an options group — a camera's Zoom, a light's Intensity — whose
/// group name has moved between versions while the property's own match name
/// has not. Direct first, so a name that appears twice (a mask's Opacity and
/// the layer's) always resolves to the one the caller was standing on.
pub(crate) fn find<'a>(props: &'a [Property], match_name: &str) -> Option<&'a Property> {
    if let Some(direct) = child(props, match_name) {
        return Some(direct);
    }
    props
        .iter()
        .find_map(|node| find(node.children(), match_name))
}

/// A leaf's static value, or one axis of it. `None` when the leaf is absent,
/// unreadable, or has fewer axes than asked for.
pub(crate) fn axis_of(value: &serde_json::Value, axis: usize) -> Option<f64> {
    match value {
        serde_json::Value::Array(items) => items.get(axis).and_then(serde_json::Value::as_f64),
        serde_json::Value::Bool(b) if axis == 0 => Some(f64::from(u8::from(*b))),
        other if axis == 0 => other.as_f64(),
        _ => None,
    }
}

/// One axis of a property, as the still number it is now. Used for the
/// switches and sizes Lumit does not animate.
pub(crate) fn still(props: &[Property], match_name: &str, axis: usize) -> Option<f64> {
    let node = find(props, match_name)?;
    match node.separated.as_deref().and_then(|f| f.get(axis)) {
        Some(follower) => follower.value.as_ref().and_then(|v| axis_of(v, 0)),
        None => node.value.as_ref().and_then(|v| axis_of(v, axis)),
    }
}

/// A mask path key's side. A path has no value graph, so the ease shapes how
/// fast the *shape* crosses to the next key rather than how fast a number
/// does; hold and linear are the two that carry, and an eased path key is
/// imported as the After Effects "easy ease" its influence describes.
pub(crate) fn path_side(interp: Option<&str>) -> SideInterp {
    match interp {
        Some("HOLD") => SideInterp::Hold,
        Some("BEZIER") => lumit_core::anim::EASY_EASE,
        _ => SideInterp::Linear,
    }
}

/// One axis of an After Effects property, as a Lumit [`LumProperty`].
///
/// `fallback` is what a property that is absent or unreadable imports as, so a
/// layer whose Opacity refused to be read still arrives opaque rather than
/// invisible.
pub(crate) fn scalar(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    props: &[Property],
    match_name: &str,
    axis: usize,
    fallback: f64,
) -> LumProperty {
    let Some(node) = find(props, match_name) else {
        return LumProperty::fixed(fallback);
    };
    from_node(conv, path, node, axis, fallback)
}

/// The same, from a node already in hand (an effect parameter, a mask's
/// opacity — the cases where the caller found the node itself).
pub(crate) fn from_node(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    node: &Property,
    axis: usize,
    fallback: f64,
) -> LumProperty {
    // A dimension-separated property keeps its animation on the followers, and
    // the leader still reports a still value — so reading the leader is not an
    // error that shows up, it is a moving layer that silently stopped moving.
    if let Some(follower) = node.separated.as_deref().and_then(|f| f.get(axis)) {
        return from_node(conv, path, follower, 0, fallback);
    }

    let name = display_name(node, match_name_of(node));
    // A property After Effects itself could not read (a CUSTOM_VALUE blob —
    // K-410). There is nothing to import, and saying so is the whole point of
    // the walker recording it rather than omitting it.
    if node.unreadable.is_some() {
        conv.report.row(
            path.property(name),
            Outcome::Skipped,
            Reason::PropertyUnreadable {
                match_name: match_name_of(node).to_string(),
            },
        );
        return LumProperty::fixed(fallback);
    }

    // An enabled expression drives the property in After Effects too, so it
    // drives it here; the keys underneath are the bundle's to keep.
    if let Some(source) = node.expression.as_deref().filter(|s| !s.trim().is_empty()) {
        if node.expression_enabled == Some(true) {
            conv.report.row(
                path.property(name),
                Outcome::Adjusted,
                Reason::ExpressionCarried,
            );
            return LumProperty {
                animation: Animation::Expression(source.to_string()),
                extra: ae_extra("expression", serde_json::json!(source)),
            };
        }
        conv.report.row(
            path.property(name),
            Outcome::Adjusted,
            Reason::ExpressionDisabledCarried,
        );
    }

    let expression_extra = node
        .expression
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map_or_else(serde_json::Map::new, |s| {
            ae_extra("expression", serde_json::json!(s))
        });

    if let Some(keys) = node.keyframes.as_deref().filter(|k| !k.is_empty()) {
        if keys.iter().any(has_spatial_tangent) {
            conv.report.row(
                path.property(name),
                Outcome::Adjusted,
                Reason::SpatialTangentsFlattened,
            );
        }
        let mut out: Vec<Keyframe> = keys
            .iter()
            .filter_map(|key| keyframe(conv, key, axis))
            .collect();
        out.sort_by_key(|k| k.time);
        out.dedup_by(|a, b| a.time == b.time);
        if !out.is_empty() {
            conv.report.imported();
            return LumProperty {
                animation: Animation::Keyframed(out),
                extra: expression_extra,
            };
        }
    }

    let value = node
        .value
        .as_ref()
        .and_then(|v| axis_of(v, axis))
        .unwrap_or(fallback);
    LumProperty {
        animation: Animation::Static(value),
        extra: expression_extra,
    }
}

/// One capture key, one axis, in the layer's own timebase.
fn keyframe(conv: &Conv<'_>, key: &AeKey, axis: usize) -> Option<Keyframe> {
    let value = key.v.as_ref().and_then(|v| axis_of(v, axis))?;
    let time = conv.layer_time(key.t?);
    Some(Keyframe {
        time,
        value,
        interp_in: side(key.in_interp.as_deref(), key.in_ease.as_deref(), axis),
        interp_out: side(key.out_interp.as_deref(), key.out_ease.as_deref(), axis),
    })
}

/// One side of one key. After Effects' influence is a percentage in 0.1–100
/// and Lumit's is a fraction in (0, 1]; the speed is value-units per second on
/// both sides and carries unchanged.
fn side(interp: Option<&str>, ease: Option<&[Ease]>, axis: usize) -> SideInterp {
    match interp {
        Some("HOLD") => SideInterp::Hold,
        Some("BEZIER") => {
            // The DOM returns one ease per dimension — except on a spatial
            // property, which returns exactly one for all of them.
            let ease = ease.and_then(|e| e.get(axis).or_else(|| e.first()));
            SideInterp::Bezier {
                speed: ease.and_then(|e| e.speed).unwrap_or(0.0),
                influence: (ease.and_then(|e| e.influence).unwrap_or(100.0 / 3.0) / 100.0)
                    .clamp(1e-3, 1.0),
            }
        }
        // LINEAR, and any interpolation name a later After Effects invents:
        // a straight line is the honest reading of "I do not know this".
        _ => SideInterp::Linear,
    }
}

fn has_spatial_tangent(key: &AeKey) -> bool {
    let live = |t: &Option<Vec<f64>>| {
        t.as_deref()
            .is_some_and(|t| t.iter().any(|c| c.abs() > 1e-9))
    };
    live(&key.in_tangent) || live(&key.out_tangent)
}

pub(crate) fn match_name_of(node: &Property) -> &str {
    node.match_name.as_deref().unwrap_or("")
}

pub(crate) fn display_name<'a>(node: &'a Property, fallback: &'a str) -> &'a str {
    node.name.as_deref().unwrap_or(fallback)
}

/// A one-key `ae` namespace map, the shape every carried-through AE fact takes.
pub(crate) fn ae_extra(
    key: &str,
    value: serde_json::Value,
) -> serde_json::Map<String, serde_json::Value> {
    ae_map(vec![(key, value)])
}

/// An `ae` namespace map from several facts.
pub(crate) fn ae_map(
    pairs: Vec<(&str, serde_json::Value)>,
) -> serde_json::Map<String, serde_json::Value> {
    let inner: serde_json::Map<String, serde_json::Value> = pairs
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .filter(|(_, v)| !v.is_null())
        .collect();
    let mut outer = serde_json::Map::new();
    if !inner.is_empty() {
        outer.insert("ae".to_string(), serde_json::Value::Object(inner));
    }
    outer
}

/// Two linear keys running `value` against `time` — the shape both the Retime
/// identity and the stretch conversion need.
pub(crate) fn ramp(from: Rational, from_v: f64, to: Rational, to_v: f64) -> LumProperty {
    let key = |time: Rational, value: f64| Keyframe {
        time,
        value,
        interp_in: SideInterp::Linear,
        interp_out: SideInterp::Linear,
    };
    LumProperty {
        animation: Animation::Keyframed(vec![key(from, from_v), key(to, to_v)]),
        extra: serde_json::Map::new(),
    }
}
