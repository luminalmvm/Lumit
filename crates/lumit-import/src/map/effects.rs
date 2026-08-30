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
//!
//! **A third-party effect is the one exception, and it has two roads** (K-655,
//! docs/11 §5). Somebody else's plug-in has internals nobody here can
//! re-implement, so an equivalent is never on offer — but two lesser things
//! are, and which one applies is a fact about the machine doing the import:
//!
//! * The user has the **vendor's own OFX build** of the same plug-in installed.
//!   Then it is not a likeness, it is the effect: the row names the plug-in's
//!   identifier, [`direct`] looks it up in the catalogue, and the controls carry
//!   across by name wherever the two sides agree on the type.
//! * They do not. Then the row names the **closest Lumit effect**, and
//!   [`nearest`] puts it in the stack at its own defaults. No dial is guessed
//!   across a vendor boundary, because a guessed dial is a silently wrong
//!   picture where a default is a visible one — the report says which effect it
//!   is standing in for, and it is dialled once.
//!
//! Both roads report. The rule that survives untouched is the one that matters:
//! nothing is ever *silently* something else.

use lumit_core::fx::ParamKind;
use lumit_core::model::{EffectInstance, EffectKey, EffectNamespace, EffectParam, EffectValue};
use uuid::Uuid;

use crate::capture::Property;
use crate::report::{ItemPath, Outcome, Reason};

use super::props::{ae_map, display_name, from_node, match_name_of};
use super::table::Row;
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
    // The two third-party roads come first, and in this order: the vendor's own
    // plug-in beats Lumit's nearest likeness, because it is the effect itself
    // (K-655). Neither road can fire on a row that names neither an `ofx`
    // identifier nor the `nearest` conversion, so Adobe's own effects reach
    // the two halves below exactly as they always have.
    if let Some(row) = super::table::table().row(match_name_of(node)) {
        if let Some(mapped) = direct(conv, path, node, row) {
            return Some(mapped);
        }
        if row.conversion == "nearest" {
            return nearest(conv, path, node, row);
        }
    }
    if let Some(mapped) = super::fx_colour::claim(conv, path, node) {
        return Some(mapped);
    }
    super::fx_distort::claim(conv, path, node)
}

/// **The vendor's own OFX build of this same plug-in, if it is installed**
/// (K-655, docs/11 §5).
///
/// The match rule is equality: the row lists the plug-in identifiers this After
/// Effects effect *is*, and one of them either answers in the catalogue this
/// session or does not. Nothing here compares labels or looks for a
/// resemblance — two products with similar names are two products, and a
/// resemblance mapped to a render is somebody else's picture with our name on
/// it. An identifier the table has wrong therefore matches nothing, and the row
/// takes [`nearest`] instead, exactly as on a machine without the plug-in.
fn direct(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    node: &Property,
    row: &Row,
) -> Option<EffectInstance> {
    let (plugin, schema) = row.ofx.iter().find_map(|id| {
        let name = format!("ofx:{id}");
        lumit_core::fx::schema(&name).map(|schema| (name, schema))
    })?;
    let mut inst = lumit_core::fx::instantiate(&plugin)?;
    // An effect switched off in After Effects imports switched off.
    inst.enabled = node.enabled.unwrap_or(true);

    let here = path.property(display_name(node, match_name_of(node)));
    let mark = conv.report.rows.len();
    let mut carried = 0usize;
    let mut controls = 0usize;
    carry(
        conv,
        path,
        &here,
        node,
        schema,
        &mut inst,
        &mut carried,
        &mut controls,
    );
    conv.report.fold_unreadable_since(mark, here.clone());

    conv.report.row(
        here,
        Outcome::Adjusted,
        Reason::EffectAsPlugin {
            match_name: match_name_of(node).to_string(),
            plugin: schema.label.to_string(),
            carried,
            controls,
        },
    );
    Some(inst)
}

/// One AE parameter leaf onto the plug-in's control of the same name.
///
/// "The same name" is the *displayed* name on both sides, folded to letters and
/// digits: After Effects numbers a third-party effect's parameters
/// (`S_Glow-0004`) where OFX keeps the plug-in's own, so the match names cannot
/// be compared and the labels are what the two builds genuinely share. The
/// type has to agree as well — a number onto a number, a colour onto a colour —
/// and a control whose type does not agree is named in the report rather than
/// coerced.
///
/// A point is deliberately **not** carried: After Effects measures one in
/// pixels and OFX in whichever canonical space the plug-in declared, so the
/// same two numbers are two different places.
/// ponytail: no point carriage until an OFX build's coordinate space is read
/// off a live installation rather than assumed; the report's carried-of-total
/// count is what shows it missing.
#[allow(clippy::too_many_arguments)]
fn carry(
    conv: &mut Conv<'_>,
    at: &ItemPath,
    here: &ItemPath,
    node: &Property,
    schema: &'static lumit_core::fx::EffectSchema,
    inst: &mut EffectInstance,
    carried: &mut usize,
    controls: &mut usize,
) {
    for leaf in node.children() {
        if leaf.group.is_some() {
            carry(conv, at, here, leaf, schema, inst, carried, controls);
            continue;
        }
        // A topic heading is not a control (the same exception the placeholder
        // road draws): counting them would make every plug-in look half-carried.
        let kind = leaf.value_type.as_deref().unwrap_or_default();
        if kind.is_empty() || kind == "group" {
            continue;
        }
        *controls = controls.saturating_add(1);

        let id = match_name_of(leaf);
        let name = display_name(leaf, id);
        let Some(param) = schema
            .params
            .iter()
            .find(|p| folded(p.label) == folded(name) || folded(p.id) == folded(name))
        else {
            continue;
        };
        let value = match (kind, &param.kind) {
            (
                "float",
                ParamKind::Float { .. }
                | ParamKind::Slider { .. }
                | ParamKind::Int { .. }
                | ParamKind::Angle { .. },
            ) => EffectValue::Float(from_node(conv, at, leaf, 0, 0.0)),
            ("colour", ParamKind::Colour { .. }) => EffectValue::Colour([
                from_node(conv, at, leaf, 0, 0.0),
                from_node(conv, at, leaf, 1, 0.0),
                from_node(conv, at, leaf, 2, 0.0),
                from_node(conv, at, leaf, 3, 1.0),
            ]),
            // Named alike, shaped unlike: said out loud rather than coerced.
            _ => {
                conv.report.row(
                    here.clone(),
                    Outcome::Adjusted,
                    Reason::EffectParamNotCarried {
                        effect: schema.label.to_string(),
                        param: name.to_string(),
                    },
                );
                continue;
            }
        };
        if let Some(p) = inst.params.iter_mut().find(|p| p.id == param.id) {
            p.value = value;
            *carried = carried.saturating_add(1);
        }
    }
}

/// A name reduced to what two builds of one plug-in genuinely share: its
/// letters and digits, in lower case.
fn folded(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// **The closest Lumit effect, at its own defaults** (K-655, docs/11 §5).
///
/// For a third-party effect with no OFX build installed. The effect arrives in
/// the stack, in the right place, switched on or off as it was — and dialled to
/// Lumit's defaults, because the vendor's numbers mean the vendor's algorithm
/// and carrying them over would be a picture that looks deliberate and is not.
/// The report names both sides so the one dial-in is somewhere the reader can
/// find it.
fn nearest(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    node: &Property,
    row: &Row,
) -> Option<EffectInstance> {
    let mut inst = lumit_core::fx::instantiate(&row.lumit)?;
    inst.enabled = node.enabled.unwrap_or(true);
    conv.report.row(
        path.property(display_name(node, match_name_of(node))),
        Outcome::Adjusted,
        Reason::EffectNearest {
            match_name: match_name_of(node).to_string(),
            instead: lumit_core::fx::schema(&row.lumit)
                .map_or_else(|| row.lumit.clone(), |s| s.label.to_string()),
        },
    );
    Some(inst)
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
    // A third-party effect refuses parameters by the dozen, and one row each
    // buries the rest of the report — so the ones raised under this instance
    // are folded into a count once the walk is done (docs/11 §9).
    let mark = conv.report.rows.len();
    for leaf in node.children() {
        collect(conv, &here, leaf, &mut params, &mut carried);
    }
    conv.report.fold_unreadable_since(mark, here.clone());

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
