//! A described plugin, turned into the declaration a built-in effect carries.
//!
//! # In plain terms
//!
//! Lumit's own effects are declared once, in a struct that says what the effect
//! is called, what family it belongs to and what its controls are
//! (docs/impl/effect-registry.md). Everything downstream — the Add-effect menu,
//! Effect controls, keyframes, expressions — reads that one declaration. A CLAP
//! plugin has just told us the same facts in CLAP's words. This module writes
//! them in Lumit's, so an audio plugin's Threshold keyframes exactly as a
//! built-in's Radius does. "The stack is the rack": there is no separate audio
//! effect list to write a second kind of declaration for
//! (docs/impl/audio-plugins.md §2).
//!
//! # The mapping decisions, written down once
//!
//! * **The row id is the plugin's own parameter id, spelled `p<number>`.**
//!   Both standards' stable key is a `u32`, not a name — plugins rename knobs across
//!   versions and reorder them freely, and §4 is explicit that the id is the
//!   key and the index never is. So `p1234` is what the project file stores
//!   and what an expression addresses. It is not pretty. The alternative —
//!   deriving the id from the name — is a saved project silently losing a
//!   keyframed value the first time a vendor rewords a label, which is worse
//!   than ugly.
//! * **Every parameter is one row.** Neither standard has vectors: a knob is a
//!   `double` between a minimum and a maximum, so nothing spreads into `_x`/`_y`
//!   the way an OFX 2-D parameter does. VST3's own values are normalised nought
//!   to one, but the range this sees is the **plain** one the controller
//!   converts to, because a plain number is what a person reads and keyframes
//!   (docs/impl/audio-plugins.md §4).
//! * **A closed range is a [`ParamKind::Slider`].** CLAP's minimum and maximum
//!   are the only legal values a parameter has, which is exactly what a Slider
//!   declares — a Float would offer a box to type a number the plugin will
//!   clamp. A **stepped** parameter is an [`ParamKind::Int`], and a stepped one
//!   whose whole range is nought to one is a [`ParamKind::Bool`], which is what
//!   a CLAP switch is.
//! * **[`Unit::Raw`], always.** CLAP carries no unit metadata at all — a
//!   parameter's units live in its `value_to_text`, which is prose for a
//!   readout rather than a dimension the resolve step could rescale. Declaring
//!   a wrong unit would be worse than declaring none.
//! * **Hidden, read-only, bypass and non-automatable parameters get no rows**
//!   (§4). They live in the state blob, which is round-tripped whole.
//! * **The plugin's `module` path becomes the panel's groups**: a run of
//!   parameters sharing a module is a run of rows behind one twirl, the way an
//!   OFX group is.
//! * **Traits.** `cost = Heavy`, because a plugin is somebody else's code and
//!   the degradation ordering should give it up first; `roi = FullFrame` and
//!   `temporal = [0]`, because an audio effect touches no picture at all and a
//!   claim about frames would be a claim about something it never sees. No
//!   matte row: a plugin's rows are its own.
//!
//! **The strings are leaked, once.** [`EffectSchema`] is a `'static`
//! declaration because a built-in's is a compile-time constant; a plugin's is
//! discovered at start-up and then lives as long as the session, so leaking it
//! is the honest spelling of that lifetime.

use lumit_core::fx::{
    CostClass, EffectSchema, EffectTraits, FxCategory, MatteRole, ParamGroup, ParamId, ParamKind,
    ParamSchema, Roi, Unit,
};

use crate::describe::{ParamDescription, PluginDescriptor, Rejection};

/// The catalogue name prefix a **CLAP** plugin's effect answers to — spelled
/// once, in the engine, so the crate that mints the name and the walks that
/// read it cannot drift (K-700). VST3's is beside it and
/// [`Abi::prefix`](crate::abi::Abi::prefix) chooses between them (K-707).
pub const MATCH_PREFIX: &str = lumit_core::fx::CLAP_MATCH_PREFIX;

/// The name one described plugin's effect answers to: its standard's prefix and
/// its own identifier.
///
/// One function, so the scan that offers the effect and the schema that declares
/// it cannot spell the same name two ways.
#[must_use]
pub fn match_name(plugin: &PluginDescriptor) -> String {
    format!("{}{}", plugin.abi.prefix(), plugin.id)
}

/// The schema row id one CLAP parameter mints.
///
/// `p` and the plugin's own stable id. See this module's note on why it is not
/// the parameter's name.
#[must_use]
pub fn row_id(param: &ParamDescription) -> String {
    format!("p{}", param.id)
}

/// One schema row's way **back** to the CLAP parameter it came from.
///
/// The trip out is [`schema_of`]; the trip home is this. A resolved bag is
/// keyed by [`ParamId`] hashes and the names are gone, so the routes are worked
/// out once, from the same enumeration that minted the rows, and nothing has to
/// guess at the reverse of the spelling rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValueRoute {
    /// The row, as the bag keys it.
    pub id: ParamId,
    /// The row's spelled id, for a report or a test to name.
    pub row: String,
    /// The plugin's own stable parameter id.
    pub param: u32,
}

/// Every schema row's route home, in schema order.
///
/// Parameters with no row — hidden, read-only, bypass, non-automatable —
/// appear nowhere here, for the same reason they appear in no bag: there is no
/// value of theirs for Lumit to carry.
#[must_use]
pub fn value_routes(plugin: &PluginDescriptor) -> Vec<ValueRoute> {
    plugin
        .rows()
        .map(|param| {
            let row = row_id(param);
            ValueRoute {
                id: ParamId::new(&row),
                row,
                param: param.id,
            }
        })
        .collect()
}

/// Turn a described plugin into the declaration Lumit's own effects carry.
///
/// # Errors
///
/// [`Rejection::DuplicateParamId`] if two rows would land on the same
/// [`ParamId`]. CLAP ids are unique by contract, so this is a plugin bug rather
/// than an expected shape — but it is a plugin bug that would otherwise show up
/// as one control quietly driving another.
pub fn schema_of(plugin: &PluginDescriptor) -> Result<EffectSchema, Rejection> {
    let mut rows: Vec<ParamSchema> = Vec::new();
    // The module path each row belongs to, in step with `rows`.
    let mut owners: Vec<String> = Vec::new();

    for param in plugin.rows() {
        rows.push(ParamSchema {
            id: leak(&row_id(param)),
            label: leak(&label_of(param)),
            kind: kind_of(param),
            unit: Unit::Raw,
        });
        owners.push(param.module.clone());
    }

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

    let groups = groups_of(&rows, &owners);
    let params: &'static [ParamSchema] = leak_slice(rows);

    Ok(EffectSchema {
        match_name: leak(&match_name(plugin)),
        label: leak(&plugin.label),
        version: major_of(&plugin.version),
        // None of Lumit's own categories is a claim about somebody else's
        // effect, and the audio category the Effects & presets panel will show
        // these under arrives with AP5's panel surface.
        // ponytail: Utility until that category exists; adding a variant now
        // would ripple into the label table and its translated strings for a
        // menu nothing yet lists.
        category: FxCategory::Utility,
        traits: EffectTraits {
            cost: CostClass::Heavy,
            roi: Roi::FullFrame,
            // An audio effect reads no other frame because it reads no frame.
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params,
        groups: leak_slice(groups),
        // A CLAP plugin greys its own controls by asking the host to rescan
        // them, which is a host extension v1 does not offer; there is nothing
        // here to translate.
        enabled_when: &[],
        matte: MatteRole::None,
    })
}

/// The row's label: the plugin's own name, or its id where it gave none.
fn label_of(param: &ParamDescription) -> String {
    if param.name.trim().is_empty() {
        format!("Parameter {}", param.id)
    } else {
        param.name.clone()
    }
}

/// What control the row draws.
fn kind_of(param: &ParamDescription) -> ParamKind {
    let (min, max, default) = (param.min, param.max, param.default);
    if !min.is_finite() || !max.is_finite() || max <= min {
        // A plugin that declares no usable range gets an unbounded number
        // rather than a slider with no travel. Not a rejection: the parameter
        // still works, it just cannot be dragged between two ends.
        return ParamKind::Float {
            default: if default.is_finite() { default } else { 0.0 },
            slider: (0.0, 1.0),
            hard: (None, None),
        };
    }
    let default = default.clamp(min, max);
    if param.stepped() {
        if min == 0.0 && max == 1.0 {
            return ParamKind::Bool {
                default: default >= 0.5,
            };
        }
        let low = min as i64;
        let high = max as i64;
        return ParamKind::Int {
            default: default as i64,
            slider: (low, high),
            hard: (Some(low), Some(high)),
        };
    }
    ParamKind::Slider {
        default,
        range: (min, max),
    }
}

/// Cut the rows into contiguous runs by the module path the plugin gave them.
///
/// A [`ParamGroup`]'s members must be a contiguous run in schema order, which
/// is how the panel draws them in place. A plugin that interleaves two modules
/// gets each stretch as its own run under the same header — the rows keep the
/// order the plugin gave them, which is the promise that matters.
fn groups_of(rows: &[ParamSchema], owners: &[String]) -> Vec<ParamGroup> {
    let mut groups: Vec<ParamGroup> = Vec::new();
    let mut run: Vec<&'static str> = Vec::new();
    let mut current = String::new();

    for (row, owner) in rows.iter().zip(owners) {
        if *owner != current {
            if !current.is_empty() {
                groups.push(group(&current, std::mem::take(&mut run)));
            }
            run.clear();
            current = owner.clone();
        }
        if !current.is_empty() {
            run.push(row.id);
        }
    }
    if !current.is_empty() {
        groups.push(group(&current, run));
    }
    groups
}

/// One twirl. The header is the last segment of the module path — a plugin
/// that says `Filter/Low pass` means the group is Low pass, inside a structure
/// Lumit's panel does not nest.
fn group(module: &str, params: Vec<&'static str>) -> ParamGroup {
    let label = module.rsplit('/').next().unwrap_or(module);
    ParamGroup {
        label: leak(label),
        params: leak_slice(params),
        collapsed: false,
        visible_when: None,
        visible_when_lens_elements: None,
    }
}

/// The leading integer of a version string, which is what the cache key wants
/// out of it.
///
/// CLAP says nothing about the shape of a version string, so this reads what is
/// there and answers 1 for anything it cannot. A vendor who bumps only a patch
/// number therefore keeps the same key — which is the same ceiling the OFX host
/// carries, and the same reason: the alternative is invalidating every cached
/// mix on a bug-fix release.
fn major_of(version: &str) -> u32 {
    version
        .split(['.', '-', ' '])
        .next()
        .and_then(|head| head.parse::<u32>().ok())
        .unwrap_or(1)
}

/// One string, for the session.
fn leak(text: &str) -> &'static str {
    Box::leak(text.to_owned().into_boxed_str())
}

/// One list, for the session.
fn leak_slice<T>(values: Vec<T>) -> &'static [T] {
    Box::leak(values.into_boxed_slice())
}
