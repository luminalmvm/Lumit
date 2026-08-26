//! A described plugin, turned into the declaration a built-in effect carries.
//!
//! # In plain terms
//!
//! Lumit's own effects are declared once, in a struct that says what the effect
//! is called, what family it belongs to, how expensive it is, and what its
//! controls are (docs/impl/effect-registry.md). Everything downstream — the
//! Add-effect menu, the Effect Controls panel, keyframes, expressions, the
//! cache key — reads that one declaration. A plugin has just told us the same
//! facts in OFX's words. This module writes them down in Lumit's, so a plugin
//! effect and a built-in are the same kind of thing to everything that comes
//! after (docs/12 §1).
//!
//! # The mapping decisions, written down once
//!
//! * **Units.** A double whose `kOfxParamPropDoubleType` is one of the
//!   *absolute* spatial kinds — `X`/`Y`/`XY`, absolute or not — is a distance,
//!   and every distance in Lumit is px@comp ([`Unit::Px`], K-419). An angle is
//!   [`Unit::Degrees`]. **Everything else is [`Unit::Raw`]**, including the
//!   normalised spatial types: a normalised coordinate runs 0 to 1 where
//!   Lumit's per cent runs 0 to 100, so drawing a "%" beside it would be a
//!   wrong unit rider rather than a missing one.
//! * **A 2-D or 3-D parameter becomes two or three rows**, `foo_x` / `foo_y` /
//!   `foo_z`. Lumit has no point *kind*: a point is two adjacent number rows
//!   the panel folds into one, which is why
//!   [`EffectSchema::pairs`](lumit_core::fx::EffectSchema::pairs) reads the
//!   suffixes (K-443). A plugin's Centre therefore draws exactly as a
//!   built-in's does, link glyph and all.
//! * **A choice keeps its option labels**, in the plugin's order, with no
//!   dividers.
//! * **A custom parameter is opaque and stays opaque** (docs/12 §2.2): it has
//!   no schema row, because there is no control that could draw a vendor blob,
//!   and it is round-tripped uninterpreted from the descriptor. Text
//!   parameters that are not paths are in the same position — Lumit has no text
//!   row — and both are listed by
//!   [`PluginDescriptor::unrepresented`](crate::describe::PluginDescriptor::unrepresented)
//!   so the omission is a line in the report rather than a silence.
//! * **Groups and pages become the panel's layout** (docs/12 §2.2). A group is
//!   a run of rows behind a twirl. A page is the same thing with a flatter
//!   name — the host advertises `kOfxParamHostPropMaxPages` = 0, so a
//!   well-behaved plugin uses groups, and a plugin that defines a page anyway
//!   gets its page drawn as a group rather than dropped.
//! * **Traits.** `cost = Heavy`, because a plugin is somebody else's code
//!   crossing a process boundary and the degradation ordering should give it up
//!   first; `roi = FullFrame`, because the host advertises no tile support and
//!   a schema that claimed a padding would be claiming it on the plugin's
//!   behalf. `temporal` follows the plugin's own declared frame access, and P5
//!   replaces it per instance from `getFramesNeeded`.
//! * **No matte row.** K-395's injected Matte, its Invert and its Channel are
//!   for **built-ins**, where the row means something the dispatch seam can
//!   carry out. A plugin's rows are its own: injecting a Matte would put a
//!   control on the panel that the plugin has never heard of and nothing would
//!   consume. So `matte = MatteRole::None`.
//!
//! **The strings are leaked, once.** [`EffectSchema`] is a `'static`
//! declaration because a built-in's is a compile-time constant; a plugin's is
//! discovered at start-up and then lives as long as the session, so leaking it
//! is the honest spelling of that lifetime rather than a leak in the sense that
//! matters. Recorded ceiling: a rescan re-leaks, so the rescan path P6 builds
//! reuses the schema it already has for an identifier and version it has
//! already seen.

use lumit_core::fx::{
    CostClass, EffectSchema, EffectTraits, FxCategory, MatteRole, ParamGroup, ParamId, ParamKind,
    ParamSchema, Roi, Unit,
};

use crate::describe::{ParamDescription, PluginDescriptor, Rejection};
use crate::ffi::{double_types, param_types, prop_keys as keys, string_modes};
use crate::props::{PropValue, PropertySet};

/// Turn a described plugin into the declaration Lumit's own effects carry.
///
/// # Errors
///
/// [`Rejection::DuplicateParamId`] if two rows would land on the same
/// [`ParamId`] — the silent collision docs/impl/effect-registry.md §5 warns
/// about, made loud here because a plugin's parameter names are not ours to
/// choose.
pub fn schema_of(plugin: &PluginDescriptor) -> Result<EffectSchema, Rejection> {
    let groups = group_owners(&plugin.params);
    let pages = page_owners(&plugin.params);

    let mut rows: Vec<ParamSchema> = Vec::new();
    // The group each row belongs to, in step with `rows`; `None` for a row at
    // the top level.
    let mut owners: Vec<Option<Owner>> = Vec::new();

    for param in &plugin.params {
        let owner = owner_of(param, &groups, &pages);
        for row in rows_of(param) {
            rows.push(row);
            owners.push(owner.clone());
        }
    }

    // Two rows under one id is two controls the panel cannot tell apart and one
    // value in the bag. Refuse the effect rather than ship the ambiguity.
    for (index, row) in rows.iter().enumerate() {
        let id = ParamId::new(row.id);
        if let Some(first) = rows[..index]
            .iter()
            .find(|other| ParamId::new(other.id) == id)
        {
            return Err(Rejection::DuplicateParamId {
                first: first.id.to_owned(),
                second: row.id.to_owned(),
            });
        }
    }

    let runs = groups_of(&rows, &owners);
    let params: &'static [ParamSchema] = leak_slice(rows);

    Ok(EffectSchema {
        match_name: leak(&format!("ofx:{}", plugin.identifier)),
        label: leak(&plugin.label),
        version: plugin.version.0,
        // A plugin's own menu path is its grouping, kept on the descriptor and
        // shown by Effects & Presets (docs/12 §2.6). None of Lumit's ten
        // categories is a claim about somebody else's effect, so the schema's
        // category is the unclassified one and the grouping does the placing.
        category: FxCategory::Utility,
        traits: EffectTraits {
            cost: CostClass::Heavy,
            roi: Roi::FullFrame,
            temporal: if plugin.temporal {
                // The plugin says it reads other frames but not yet *which* —
                // `getFramesNeeded` answers that per instance, and P5 asks it.
                // Widening is the safe direction meanwhile: an offset we
                // include and the plugin never asks for costs one hash input,
                // one we leave out costs a wrong frame served from the cache.
                &[-1, 0, 1]
            } else {
                &[0]
            },
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params,
        groups: leak_slice(runs),
        // Greying rules are a built-in's declaration about its own controls.
        // OFX has no equivalent a host may read at describe time — a plugin
        // greys its own rows by calling back during `instanceChanged` — so
        // there is nothing here to translate.
        enabled_when: &[],
        matte: MatteRole::None,
    })
}

/// What a group or page is called, and whether it starts closed.
#[derive(Clone, PartialEq, Eq)]
struct Owner {
    /// The parameter name of the group, or the page. The identity, not the
    /// label: two groups may share a label.
    name: String,
    /// The twirl header.
    label: String,
    /// Whether the twirl starts closed.
    collapsed: bool,
}

/// Every group parameter, as the twirl it becomes.
///
/// A group is a parameter like any other in OFX: its label is the header, and
/// `kOfxParamPropGroupOpen` says whether it starts open. Reading them first is
/// what lets a member name its group by the group's *parameter name* — which
/// is what `kOfxParamPropParent` holds — and still get the group's label drawn.
fn group_owners(params: &[ParamDescription]) -> Vec<Owner> {
    params
        .iter()
        .filter(|param| param.param_type == param_types::GROUP)
        .map(|group| Owner {
            name: group.name.clone(),
            label: label_of(&group.props, &group.name),
            collapsed: int_at(&group.props, keys::PARAM_GROUP_OPEN, 0) == Some(0),
        })
        .collect()
}

/// Which page lists each parameter, by parameter name.
fn page_owners(params: &[ParamDescription]) -> Vec<(String, Owner)> {
    let mut owners = Vec::new();
    for page in params
        .iter()
        .filter(|param| param.param_type == param_types::PAGE)
    {
        let owner = Owner {
            name: page.name.clone(),
            label: label_of(&page.props, &page.name),
            collapsed: false,
        };
        for child in strings(&page.props, keys::PARAM_PAGE_CHILD) {
            // A page's child list carries layout sentinels as well as names;
            // anything that is not a parameter simply names no parameter.
            owners.push((child, owner.clone()));
        }
    }
    owners
}

/// The group a parameter belongs to: its own group parent if it has one, and
/// otherwise the page that lists it.
fn owner_of(
    param: &ParamDescription,
    groups: &[Owner],
    pages: &[(String, Owner)],
) -> Option<Owner> {
    let parent = string_at(&param.props, keys::PARAM_PARENT, 0).unwrap_or_default();
    if !parent.is_empty() {
        // A parent that names no group is a plugin bug; the row then belongs
        // to nothing, which draws it at the top level rather than losing it.
        return groups.iter().find(|group| group.name == parent).cloned();
    }
    pages
        .iter()
        .find(|(child, _)| *child == param.name)
        .map(|(_, owner)| owner.clone())
}

/// Fill in each group's label and open state from the group parameter itself,
/// then cut the rows into contiguous runs.
///
/// A [`ParamGroup`]'s members must be a contiguous run in schema order, which
/// is how the panel draws them in place. A plugin that interleaves two groups
/// gets each stretch as its own run under the same header — the rows keep the
/// order the plugin gave them, which is the promise that matters.
fn groups_of(rows: &[ParamSchema], owners: &[Option<Owner>]) -> Vec<ParamGroup> {
    let mut groups: Vec<ParamGroup> = Vec::new();
    let mut run: Vec<&'static str> = Vec::new();
    let mut current: Option<Owner> = None;

    for (row, owner) in rows.iter().zip(owners) {
        if owner.as_ref() != current.as_ref() {
            if let Some(owner) = current.take() {
                groups.push(group(&owner, std::mem::take(&mut run)));
            }
            current = owner.clone();
        }
        if current.is_some() {
            run.push(row.id);
        }
    }
    if let Some(owner) = current {
        groups.push(group(&owner, run));
    }
    groups
}

fn group(owner: &Owner, params: Vec<&'static str>) -> ParamGroup {
    ParamGroup {
        label: leak(&owner.label),
        params: leak_slice(params),
        collapsed: owner.collapsed,
        visible_when: None,
        visible_when_lens_elements: None,
    }
}

/// The schema rows one OFX parameter becomes: one for a scalar, two or three
/// for a point, none for the kinds Lumit has no control to draw.
fn rows_of(param: &ParamDescription) -> Vec<ParamSchema> {
    let props = &param.props;
    let label = label_of(props, &param.name);
    let unit = unit_of(props);

    let axes: &[&str] = &["x", "y", "z"];
    let axis_labels: &[&str] = &["X", "Y", "Z"];

    /// One row per component, `foo_x` / `foo_y` / `foo_z`.
    fn spread(
        name: &str,
        label: &str,
        unit: Unit,
        count: usize,
        axes: &[&str],
        axis_labels: &[&str],
        kind: impl Fn(usize) -> ParamKind,
    ) -> Vec<ParamSchema> {
        (0..count)
            .map(|index| ParamSchema {
                id: leak(&format!(
                    "{name}_{}",
                    axes.get(index).copied().unwrap_or("n")
                )),
                label: leak(&format!(
                    "{label} {}",
                    axis_labels.get(index).copied().unwrap_or("")
                )),
                kind: kind(index),
                unit,
            })
            .collect()
    }

    let one = |kind: ParamKind, unit: Unit| {
        vec![ParamSchema {
            id: leak(&param.name),
            label: leak(&label),
            kind,
            unit,
        }]
    };

    match param.param_type.as_str() {
        param_types::DOUBLE => {
            if unit == Unit::Degrees {
                return one(
                    ParamKind::Angle {
                        default: double_at(props, keys::PARAM_DEFAULT, 0).unwrap_or(0.0),
                        dial_step: 1.0,
                    },
                    Unit::Degrees,
                );
            }
            one(double_kind(props, 0), unit)
        }
        param_types::DOUBLE_2D | param_types::DOUBLE_3D => {
            let count = if param.param_type == param_types::DOUBLE_2D {
                2
            } else {
                3
            };
            spread(
                &param.name,
                &label,
                unit,
                count,
                axes,
                axis_labels,
                |index| double_kind(props, index),
            )
        }
        param_types::INTEGER => one(int_kind(props, 0), unit),
        param_types::INTEGER_2D | param_types::INTEGER_3D => {
            let count = if param.param_type == param_types::INTEGER_2D {
                2
            } else {
                3
            };
            spread(
                &param.name,
                &label,
                unit,
                count,
                axes,
                axis_labels,
                |index| int_kind(props, index),
            )
        }
        param_types::BOOLEAN => one(
            ParamKind::Bool {
                default: int_at(props, keys::PARAM_DEFAULT, 0).unwrap_or(0) != 0,
            },
            Unit::Raw,
        ),
        param_types::CHOICE => {
            let options: Vec<&'static str> = strings(props, keys::PARAM_CHOICE_OPTION)
                .iter()
                .map(|option| leak(option))
                .collect();
            one(
                ParamKind::Choice {
                    options: leak_slice(options),
                    default: u32::try_from(int_at(props, keys::PARAM_DEFAULT, 0).unwrap_or(0))
                        .unwrap_or(0),
                    dividers_after: &[],
                },
                Unit::Raw,
            )
        }
        param_types::RGB | param_types::RGBA => {
            let channel = |index: usize, fallback: f64| {
                double_at(props, keys::PARAM_DEFAULT, index).unwrap_or(fallback)
            };
            let alpha = if param.param_type == param_types::RGBA {
                channel(3, 1.0)
            } else {
                1.0
            };
            one(
                ParamKind::Colour {
                    default: [channel(0, 0.0), channel(1, 0.0), channel(2, 0.0), alpha],
                    range: (
                        double_at(props, keys::PARAM_MIN, 0)
                            .filter(|value| value.is_finite() && *value > -1e30)
                            .unwrap_or(0.0),
                        double_at(props, keys::PARAM_MAX, 0)
                            .filter(|value| value.is_finite() && *value < 1e30)
                            .unwrap_or(1.0),
                    ),
                },
                Unit::Raw,
            )
        }
        param_types::STRING if string_is_path(props) => {
            // A path is a path: Lumit already draws one, with a dialog behind
            // it (K-111).
            one(
                ParamKind::File {
                    filter: &[],
                    filter_name: "All files",
                },
                Unit::Raw,
            )
        }
        param_types::PUSH_BUTTON => one(ParamKind::Action, Unit::Raw),
        // Group and page are layout, not values; custom is an opaque vendor
        // blob (docs/12 §2.2); a string that is not a path is text, and Lumit
        // has no text row; parametric is a function rather than the control
        // points Lumit's curve is made of (K-412).
        _ => Vec::new(),
    }
}

/// Whether a string parameter is a path, and therefore a
/// [`ParamKind::File`] rather than text.
pub(crate) fn string_is_path(props: &PropertySet) -> bool {
    let mode = string_at(props, keys::PARAM_STRING_MODE, 0).unwrap_or_default();
    mode == string_modes::FILE_PATH || mode == string_modes::DIRECTORY_PATH
}

/// A double component's kind: the slider is the plugin's display range, the
/// hard bounds are its real ones, and a bound at the type's own extreme is no
/// bound at all.
fn double_kind(props: &PropertySet, index: usize) -> ParamKind {
    let default = double_at(props, keys::PARAM_DEFAULT, index).unwrap_or(0.0);
    let hard = (
        double_at(props, keys::PARAM_MIN, index)
            .filter(|value| value.is_finite() && *value > -1e30),
        double_at(props, keys::PARAM_MAX, index).filter(|value| value.is_finite() && *value < 1e30),
    );
    let slider = (
        double_at(props, keys::PARAM_DISPLAY_MIN, index).unwrap_or(hard.0.unwrap_or(0.0)),
        double_at(props, keys::PARAM_DISPLAY_MAX, index).unwrap_or(hard.1.unwrap_or(1.0)),
    );
    ParamKind::Float {
        default,
        slider,
        hard,
    }
}

/// The same for a whole number.
fn int_kind(props: &PropertySet, index: usize) -> ParamKind {
    let default = i64::from(int_at(props, keys::PARAM_DEFAULT, index).unwrap_or(0));
    let hard = (
        int_at(props, keys::PARAM_MIN, index)
            .filter(|value| *value != i32::MIN && *value != -i32::MAX)
            .map(i64::from),
        int_at(props, keys::PARAM_MAX, index)
            .filter(|value| *value != i32::MAX)
            .map(i64::from),
    );
    let slider = (
        int_at(props, keys::PARAM_DISPLAY_MIN, index).map_or(hard.0.unwrap_or(0), i64::from),
        int_at(props, keys::PARAM_DISPLAY_MAX, index).map_or(hard.1.unwrap_or(100), i64::from),
    );
    ParamKind::Int {
        default,
        slider,
        hard,
    }
}

/// What a double *means*, and therefore what unit it is in.
fn unit_of(props: &PropertySet) -> Unit {
    match string_at(props, keys::PARAM_DOUBLE_TYPE, 0)
        .unwrap_or_default()
        .as_str()
    {
        double_types::ANGLE => Unit::Degrees,
        double_types::X
        | double_types::X_ABSOLUTE
        | double_types::Y
        | double_types::Y_ABSOLUTE
        | double_types::XY
        | double_types::XY_ABSOLUTE => Unit::Px,
        _ => Unit::Raw,
    }
}

/// The label a person sees, or the parameter's own name if the plugin gave it
/// none.
fn label_of(props: &PropertySet, name: &str) -> String {
    for key in [keys::LABEL, keys::LONG_LABEL, keys::SHORT_LABEL] {
        if let Ok(text) = props.get_string(key, 0) {
            let text = text.to_string_lossy();
            if !text.is_empty() {
                return text.into_owned();
            }
        }
    }
    name.to_owned()
}

fn double_at(props: &PropertySet, key: &str, index: usize) -> Option<f64> {
    props.get_double(key, index).ok()
}

fn int_at(props: &PropertySet, key: &str, index: usize) -> Option<i32> {
    props.get_int(key, index).ok()
}

fn string_at(props: &PropertySet, key: &str, index: usize) -> Option<String> {
    props
        .get_string(key, index)
        .ok()
        .map(|text| text.to_string_lossy().into_owned())
}

fn strings(props: &PropertySet, key: &str) -> Vec<String> {
    match props.get(key) {
        Ok(PropValue::String(values)) => values
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect(),
        _ => Vec::new(),
    }
}

/// One string, for the session (see the module header).
fn leak(text: &str) -> &'static str {
    Box::leak(text.to_owned().into_boxed_str())
}

/// One list, for the session.
fn leak_slice<T>(values: Vec<T>) -> &'static [T] {
    Box::leak(values.into_boxed_slice())
}
