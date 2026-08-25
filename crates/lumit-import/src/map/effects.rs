//! Effect instances, and the seam the mapping table plugs into
//! ([docs/11-AE-IMPORT.md](../../../../docs/11-AE-IMPORT.md) §5 and §6).
//!
//! # In plain terms
//!
//! An After Effects effect arrives as a group of properties under a *match
//! name* — `ADBE Gaussian Blur 2`, the stable internal name that survives the
//! user renaming things. Two futures are open to it. Either the mapping table
//! recognises the name and builds the Lumit effect that does the same job, or
//! nothing recognises it and it becomes a **placeholder**: an inert node that
//! renders nothing, keeps its name, its on/off state and every one of its
//! parameters as real animatable Lumit properties, and is never silently
//! dropped or guessed at.
//!
//! [`claim`] is the seam between the two: it asks each half of the mapping
//! table (`fx_colour`, then `fx_distort`) in turn, and a match
//! name neither claims takes the placeholder road. That fall-through is the
//! rule rather than a gap — docs/11 §5 is explicit that an unmapped match name
//! becomes a placeholder and **never the closest guess**.

use lumit_core::model::{EffectInstance, EffectKey, EffectNamespace, EffectParam, EffectValue};
use uuid::Uuid;

use crate::capture::Property;
use crate::report::{ItemPath, Outcome, Reason};

use super::props::{ae_map, display_name, from_node, match_name_of};
use super::Conv;

/// What became of one effect instance.
///
/// The two arms carry the same type on purpose: a placeholder *is* an ordinary
/// [`EffectInstance`], distinguished only by its namespace
/// ([`EffectNamespace::Placeholder`], which the resolver already renders as
/// identity). The enum exists so the caller can count the two apart for the
/// report without inspecting the namespace.
#[derive(Debug, Clone, PartialEq)]
pub enum MappedEffect {
    /// The table recognised the match name and built the Lumit equivalent.
    Mapped(EffectInstance),
    /// Nothing claimed the match name; the instance is inert and complete.
    Placeholder(EffectInstance),
}

impl MappedEffect {
    /// The instance either way — what the layer's stack actually receives.
    #[must_use]
    pub fn instance(self) -> EffectInstance {
        match self {
            Self::Mapped(e) | Self::Placeholder(e) => e,
        }
    }
}

/// One After Effects effect instance, mapped or placeheld.
///
/// **This is the seam the effect-mapping stage implements against.** It takes
/// the capture node exactly as the walker recorded it — the effect group, with
/// its match name, its display name, its enabled flag and its whole parameter
/// subtree — plus the conversion context that owns the composition's timebase
/// and the report, and the path a report row is filed under. Adding the table
/// means giving [`claim`] a body; nothing else here changes.
pub fn map_effect(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> MappedEffect {
    match claim(conv, path, node) {
        Some(mapped) => MappedEffect::Mapped(mapped),
        None => MappedEffect::Placeholder(placeholder(conv, path, node)),
    }
}

/// The mapping table's claim on a match name — docs/11 §5.
///
/// Returns the Lumit instance when the table knows this effect, and `None`
/// when it does not, which is what sends the instance down the placeholder
/// road. The table is split in two halves by category so that neither can
/// collide with the other; each is asked in turn and the first to claim the
/// name wins.
fn claim(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    if let Some(mapped) = super::fx_colour::claim(conv, path, node) {
        return Some(mapped);
    }
    super::fx_distort::claim(conv, path, node)
}

/// An inert instance that keeps everything (docs/11 §6).
///
/// The animatable leaves — floats, points, colours — become real Lumit
/// properties, so they animate, show in the graph editor and are
/// expression-readable exactly as §6 requires; they simply drive nothing. The
/// leaves Lumit has no property shape for (After Effects' unreadable custom
/// blobs, layer and mask references, text documents) are kept verbatim in the
/// instance's `ae` namespace, which `.lum` carries through load and save
/// untouched.
fn placeholder(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> EffectInstance {
    let match_name = match_name_of(node).to_string();
    let name = display_name(node, &match_name).to_string();
    let here = path.property(&name);

    let mut params = Vec::new();
    let mut carried = Vec::new();
    for leaf in node.children() {
        collect(conv, &here, leaf, &mut params, &mut carried);
    }

    conv.report.row(
        here,
        Outcome::Placeholder,
        Reason::EffectPlaceholder {
            match_name: match_name.clone(),
        },
    );

    EffectInstance {
        id: Uuid::now_v7(),
        effect: EffectKey {
            namespace: EffectNamespace::Placeholder,
            match_name,
            version: 0,
            extra: serde_json::Map::new(),
        },
        // An effect switched off in After Effects imports switched off.
        enabled: node.enabled.unwrap_or(true),
        params,
        sample_temporally: true,
        // docs/11 §6: the placeholder keeps the name the user was looking at,
        // which is what `custom_name` is for (K-321).
        custom_name: Some(name),
        // Nothing to link: a placeholder carries no schema, so it has no
        // `_x`/`_y` pairs for a chain to tie together (K-443).
        linked_pairs: Vec::new(),
        extra: ae_map(vec![("params", serde_json::Value::Array(carried))]),
    }
}

/// One parameter leaf (or one group of them — AE nests, and a placeholder's
/// stack is flat, so the walk flattens by match name).
fn collect(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    node: &Property,
    params: &mut Vec<EffectParam>,
    carried: &mut Vec<serde_json::Value>,
) {
    if node.group.is_some() {
        for child in node.children() {
            collect(conv, path, child, params, carried);
        }
        return;
    }

    let id = match_name_of(node).to_string();
    if id.is_empty() {
        return;
    }
    let value = match node.value_type.as_deref() {
        Some("float") => Some(EffectValue::Float(from_node(conv, path, node, 0, 0.0))),
        Some("point") | Some("point3") => Some(EffectValue::Point(
            from_node(conv, path, node, 0, 0.0),
            from_node(conv, path, node, 1, 0.0),
        )),
        Some("colour") => Some(EffectValue::Colour([
            from_node(conv, path, node, 0, 0.0),
            from_node(conv, path, node, 1, 0.0),
            from_node(conv, path, node, 2, 0.0),
            from_node(conv, path, node, 3, 1.0),
        ])),
        _ => None,
    };

    match value {
        Some(value) => params.push(EffectParam {
            id,
            value,
            extra: serde_json::Map::new(),
        }),
        // Nothing Lumit animates: kept whole rather than approximated.
        None => {
            // Raising the row is `from_node`'s job for the animatable leaves;
            // this branch is the only place an unreadable non-numeric leaf
            // would otherwise go unmentioned.
            //
            // **A group is not a parameter**, though, and that exception is
            // load-bearing rather than tidy-minded: an effect's topic headings
            // and its declared-empty slots are unreadable in exactly the sense
            // that there was never anything to read, and a plug-in-heavy
            // project has thousands of them (one real project: 1,907 of them
            // against 509 rows worth reading). A report nobody can scroll
            // through says nothing at all (docs/11 §9), and none of those rows
            // named a single thing the user lost.
            if node.unreadable.is_some() && node.value_type.as_deref() != Some("group") {
                conv.report.row(
                    path.property(display_name(node, &id)),
                    Outcome::Skipped,
                    Reason::PropertyUnreadable {
                        match_name: id.clone(),
                    },
                );
            }
            carried.push(serde_json::json!({
                "match_name": id,
                "name": node.name,
                "value_type": node.value_type,
                "value": node.value,
                "expression": node.expression,
                "unreadable": node.unreadable,
            }));
        }
    }
}
