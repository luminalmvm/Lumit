//! What a plugin declared, collected as it declares it (docs/impl/lfx.md §2.2).
//!
//! # In plain terms
//!
//! Describe is a **sink**, not a property bag. The host hands the plugin
//! somewhere to push typed records into - one call per kind, each carrying a
//! range, a default, a unit and some flags - and the plugin pushes them in the
//! order it wants them drawn. There is no key, no string-valued answer, and no
//! question the host can ask that a plugin can answer in the wrong type.
//!
//! This module is the host's own side of that sink: [`Describe`] collects the
//! records, answers each push `true` or `false`, and hands back a
//! [`Described`] - the declarations in order, the headings over them, and the
//! report lines for everything it declined. What turns those into the
//! declaration a built-in carries is [`crate::schema`].
//!
//! # Two kinds of no
//!
//! A push answers `false` for both, and the return value deliberately does not
//! tell them apart, because a plugin's sensible response is the same either
//! way: carry on declaring. A **row** this build cannot draw - a text row, a
//! bezier path, a dropdown with nothing in it - costs that row its control and
//! nothing else; the effect still loads and the row keeps the default it was
//! declared with, for the effect's life. A **structural** fault - two rows that
//! would drive each other, a unit nobody stated - refuses the whole effect, and
//! [`Describe::finish`] is where that is said. Carrying on after one is allowed
//! and costs nothing; it is simply not always enough to save the effect.
//!
//! # Where the C ABI meets this
//!
//! Nowhere, on purpose. `lfx_describe_sink`'s thirteen declaration calls and
//! two heading calls are filled in by whoever is holding the plugin - the
//! in-process host in tests, the broker in the shipping path - and each of them
//! converts one `*const lfx_*_param` into a [`Declaration`] and calls
//! [`Describe::declare`]. The conversions that are not field for field live
//! here too ([`Declared::float_from_abi`] and its three siblings), so that the
//! two edges cannot read the same frozen struct two different ways. Keeping the
//! raw pointers out of here is what lets the whole of the lowering be tested
//! with no plugin, no process and no `unsafe` at all.
//!
//! # Thread role
//!
//! Control thread, and only from inside the plugin's `describe`. A
//! [`Describe`] is not shared and has no interior mutability.

use std::collections::HashMap;

use lumit_core::fx::{ParamId, Unit};
use lumit_lfx_abi::{
    LfxAlpha, LfxBoolParam, LfxBounds, LfxCategory, LfxCost, LfxFloatParam, LfxIntParam,
    LfxParamFlags, LfxParamKind, LfxRoiKind, LfxSliderParam, LfxTraitFlags, LfxTraits,
    LFX_BOUND_MAX, LFX_BOUND_MIN, LFX_MAX_CURVE_POINTS, LFX_MAX_DIVIDERS, LFX_MAX_FILTERS,
    LFX_MAX_OPTIONS, LFX_MAX_PARAMS, LFX_MAX_STRING_BYTES, LFX_MIN_CURVE_POINTS, LFX_PARAM_ACTION,
    LFX_PARAM_ANGLE, LFX_PARAM_BOOL, LFX_PARAM_CHOICE, LFX_PARAM_COLOUR, LFX_PARAM_CURVE,
    LFX_PARAM_FILE, LFX_PARAM_FLAG_HIDDEN, LFX_PARAM_FLAG_STATIC, LFX_PARAM_FLOAT, LFX_PARAM_INT,
    LFX_PARAM_PATH, LFX_PARAM_POINT2, LFX_PARAM_POINT3, LFX_PARAM_SEED, LFX_PARAM_SLIDER,
    LFX_PARAM_STRING,
};
use serde::{Deserialize, Serialize};

use crate::{schema, LfxRejection};

/// What one control is, beyond the four words every declaration opens with.
///
/// One variant per `declare_*` call the sink offers, carrying the same fields
/// the C struct does in the same spellings, so the conversion at the ABI edge
/// is field for field almost everywhere.
///
/// **Almost**, and the exceptions are why [`Declared::float_from_abi`] and its
/// three siblings live here. Three fields are a decision rather than a copy: a
/// Float's and an Int's `bounds` mask says which of `hard_min` and `hard_max`
/// are meant at all, a Slider's `log` is a `uint32_t` where this is a `bool`,
/// and a Bool's `default_value` is a `uint32_t` the header documents as "`0` or
/// `1`; nothing else is a boolean". Each is a decision two edges would
/// otherwise take separately - the in-process host and the broker - and
/// take differently, so each is taken once, here, and pinned by
/// `a_declaration_crosses_the_numbers_it_was_written_with`.
///
/// The kinds this version reserves are **not** here. A `PATH` and a `STRING`
/// never become a declaration at all ([`Describe::decline_kind`]), which keeps
/// every match over this enum free of an arm for a control that cannot exist.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum Declared {
    /// An unbounded number, with optional hard bounds.
    Float {
        /// What a fresh instance starts at.
        default: f64,
        /// The slider's travel, which typing may exceed.
        slider: (f64, f64),
        /// The bounds typing may not exceed, either side open.
        hard: (Option<f64>, Option<f64>),
    },
    /// A bounded number: the range is the parameter's whole nature.
    Slider {
        /// What a fresh instance starts at.
        default: f64,
        /// The closed range - the travel and the hard bounds at once.
        range: (f64, f64),
        /// Whether the thumb moves through it logarithmically.
        log: bool,
    },
    /// A whole number.
    Int {
        /// What a fresh instance starts at.
        default: i64,
        /// The slider's travel.
        slider: (i64, i64),
        /// The bounds typing may not exceed, either side open.
        hard: (Option<i64>, Option<i64>),
    },
    /// An angle in degrees, drawn as a dial and deliberately unbounded.
    Angle {
        /// What a fresh instance starts at, in degrees.
        default: f64,
        /// The snapping increment while a modifier is held, in degrees.
        dial_step: f64,
    },
    /// A switch.
    Bool {
        /// What a fresh instance starts at.
        default: bool,
    },
    /// A dropdown, with its dividers declared rather than guessed.
    Choice {
        /// The labels, in menu order.
        options: Vec<String>,
        /// The option a fresh instance starts on.
        default: u32,
        /// The indices after which the list draws a rule.
        dividers_after: Vec<u32>,
    },
    /// Scene-linear RGBA.
    Colour {
        /// What a fresh instance starts at.
        default: [f64; 4],
        /// The low and high ends of each channel's edit range.
        range: (f64, f64),
    },
    /// An integer seed, whose default the host draws from the instance's own
    /// id rather than from the declaration.
    Seed,
    /// A point, which becomes the rows `<id>_x` and `<id>_y`.
    Point2 {
        /// Where a fresh instance's point sits.
        default: (f64, f64),
        /// The slider travel both axes share.
        slider: (f64, f64),
    },
    /// A point in three dimensions, which becomes three rows.
    Point3 {
        /// Where a fresh instance's point sits.
        default: (f64, f64, f64),
        /// The slider travel all three axes share.
        slider: (f64, f64),
    },
    /// A **tone** curve: points in the unit square, static in version 1.
    Curve {
        /// The shape a fresh instance starts with.
        default: Vec<[f32; 2]>,
    },
    /// A file chosen from a dialog, whose payload rides beside the op.
    File {
        /// The extensions the dialog offers, lower case and without the dot.
        filter: Vec<String>,
        /// What the dialog calls that set of extensions.
        filter_name: String,
    },
    /// A button: no value, no keyframe, nothing in the bag.
    Action,
}

impl Declared {
    /// The frozen ABI's own tag for this kind - the number the value's element
    /// carries, and therefore which arm of the union it is read through.
    #[must_use]
    pub const fn tag(&self) -> LfxParamKind {
        match self {
            Declared::Float { .. } => LFX_PARAM_FLOAT,
            Declared::Slider { .. } => LFX_PARAM_SLIDER,
            Declared::Int { .. } => LFX_PARAM_INT,
            Declared::Angle { .. } => LFX_PARAM_ANGLE,
            Declared::Bool { .. } => LFX_PARAM_BOOL,
            Declared::Choice { .. } => LFX_PARAM_CHOICE,
            Declared::Colour { .. } => LFX_PARAM_COLOUR,
            Declared::Seed => LFX_PARAM_SEED,
            Declared::Point2 { .. } => LFX_PARAM_POINT2,
            Declared::Point3 { .. } => LFX_PARAM_POINT3,
            Declared::Curve { .. } => LFX_PARAM_CURVE,
            Declared::File { .. } => LFX_PARAM_FILE,
            Declared::Action => LFX_PARAM_ACTION,
        }
    }

    /// The axis suffixes this kind spreads over, or `None` for the kinds that
    /// are one row.
    ///
    /// Lumit has deliberately no Point kind: a point is two adjacent number
    /// rows the panel folds into one crosshair, found by
    /// [`EffectSchema::pairs`](lumit_core::fx::EffectSchema::pairs) reading the
    /// suffixes. This is the one place the suffixes are spelled, which is what
    /// lets [`crate::schema::value_routes`] reverse them by reading the same
    /// list rather than by guessing at the rule.
    #[must_use]
    pub const fn axes(&self) -> Option<&'static [&'static str]> {
        match self {
            Declared::Point2 { .. } => Some(&["x", "y"]),
            Declared::Point3 { .. } => Some(&["x", "y", "z"]),
            _ => None,
        }
    }

    /// An unbounded number, read off the frozen struct.
    ///
    /// The one decision is `bounds`: the header says `hard_min` and `hard_max`
    /// are "read only if `bounds` says so", so a bound the mask does not claim
    /// is an **open** side rather than whatever happened to be in the field.
    /// A vendor's `lfx_float_param p = {0};` with two fields filled in then
    /// declares no hard bounds, rather than a control pinned to nought.
    #[must_use]
    pub const fn float_from_abi(declared: &LfxFloatParam) -> Self {
        Declared::Float {
            default: declared.default_value,
            slider: (declared.slider_min, declared.slider_max),
            hard: (
                bound(declared.bounds, LFX_BOUND_MIN, declared.hard_min),
                bound(declared.bounds, LFX_BOUND_MAX, declared.hard_max),
            ),
        }
    }

    /// A whole number, read off the frozen struct. `bounds` again.
    #[must_use]
    pub const fn int_from_abi(declared: &LfxIntParam) -> Self {
        Declared::Int {
            default: declared.default_value,
            slider: (declared.slider_min, declared.slider_max),
            hard: (
                whole_bound(declared.bounds, LFX_BOUND_MIN, declared.hard_min),
                whole_bound(declared.bounds, LFX_BOUND_MAX, declared.hard_max),
            ),
        }
    }

    /// A bounded number, read off the frozen struct.
    ///
    /// The one decision is `log`, which is a `uint32_t` the header describes as
    /// "non-zero: the thumb moves through the range logarithmically". Non-zero,
    /// and not "equal to one": a C author writing `p.log = 2` meant yes.
    #[must_use]
    pub const fn slider_from_abi(declared: &LfxSliderParam) -> Self {
        Declared::Slider {
            default: declared.default_value,
            range: (declared.range_min, declared.range_max),
            log: declared.log != 0,
        }
    }

    /// A switch, read off the frozen struct.
    ///
    /// The header says the default is "`0` or `1`; nothing else is a boolean",
    /// and the reading taken here is C's own: anything but nought is `true`.
    /// The alternative - treating a 2 as `false` - would make a plugin that
    /// wrote a truthy number start switched off, which is the surprising
    /// answer to a declaration that was merely sloppy.
    #[must_use]
    pub const fn bool_from_abi(declared: &LfxBoolParam) -> Self {
        Declared::Bool {
            default: declared.default_value != 0,
        }
    }
}

/// One side of a Float's hard range: the number, or `None` where the `bounds`
/// mask does not claim it.
const fn bound(bounds: LfxBounds, side: LfxBounds, value: f64) -> Option<f64> {
    if bounds & side == 0 {
        None
    } else {
        Some(value)
    }
}

/// The same for a whole number.
const fn whole_bound(bounds: LfxBounds, side: LfxBounds, value: i64) -> Option<i64> {
    if bounds & side == 0 {
        None
    } else {
        Some(value)
    }
}

/// One control, as the plugin declared it and the host wrote it down.
///
/// The unit is already Lumit's own: `LFX_UNIT_UNSET` and every number outside
/// the enumeration lower to [`Unit::Unset`], which is what [`Describe`] refuses
/// on the way in, so nothing downstream has to carry the raw number or decide
/// again what it meant.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Declaration {
    /// snake_case, stable, and hashed to the host's own parameter id.
    pub id: String,
    /// Sentence case, and what a person reads.
    pub label: String,
    /// What the number means.
    pub unit: Unit,
    /// Static, hidden - the two departures from the ordinary row, read by
    /// [`Declaration::is_static`] and [`Declaration::hidden`].
    pub flags: LfxParamFlags,
    /// What kind of control it is, and the numbers that go with it.
    pub kind: Declared,
}

impl Declaration {
    /// Whether the plugin declared this row hidden: kept, serialised, and not
    /// drawn.
    #[must_use]
    pub const fn hidden(&self) -> bool {
        self.flags & LFX_PARAM_FLAG_HIDDEN != 0
    }

    /// Whether the plugin declared this row static: one value for the whole of
    /// the effect's life, as a file choice or a tone curve is.
    ///
    /// Read, and - on a kind this build animates - not honoured:
    /// [`ParamSchema`](lumit_core::fx::ParamSchema) carries no non-animatable
    /// field, so the row is drawn keyframeable whatever the declaration said
    /// and [`LfxRejection::StaticRowAnimatesAnyway`] is the sentence the
    /// vendor's own validator run prints.
    #[must_use]
    pub const fn is_static(&self) -> bool {
        self.flags & LFX_PARAM_FLAG_STATIC != 0
    }
}

/// One heading and the run of declarations under it.
///
/// A [`ParamGroup`](lumit_core::fx::ParamGroup)'s members must be a contiguous
/// run in schema order, and a `group_begin` / `group_end` pair is contiguous by
/// construction - which is why the sink declines a heading opened inside
/// another rather than flattening it afterwards.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GroupRun {
    /// snake_case, stable.
    pub id: String,
    /// The twirl header.
    pub label: String,
    /// Whether the whole run is hidden.
    pub hidden: bool,
    /// The first declaration in the run, as an index into
    /// [`Described::params`].
    pub first: usize,
    /// How many declarations are in it. Never nought: a heading over no rows
    /// is dropped rather than drawn empty.
    pub len: usize,
}

/// What one `describe` produced.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Described {
    /// The controls, in the order the plugin declared them.
    pub params: Vec<Declaration>,
    /// The headings over them, in the order they opened.
    pub groups: Vec<GroupRun>,
    /// Everything the sink declined, each as the sentence the scan report
    /// prints. Every one of these answers `false` to
    /// [`LfxRejection::refuses_the_effect`]; the one that does not is returned
    /// by [`Describe::finish`] instead.
    pub report: Vec<LfxRejection>,
}

/// What the host schedules an effect from, as the descriptor declared it.
///
/// A plain owned mirror of `lfx_traits`, kept in the ABI's own numbers rather
/// than lowered on arrival: the lowering is [`crate::schema::traits_of`], which
/// is where "unstated means pessimistic" is written down once and tested.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Traits {
    /// What one frame costs; unset lowers to heavy.
    pub cost: LfxCost,
    /// How far past an output pixel the effect reads; unset lowers to full
    /// frame.
    pub roi_kind: LfxRoiKind,
    /// The dilation a padded region asks for, in px@comp.
    pub roi_padding_px: f32,
    /// The first frame read, relative to the one being rendered.
    pub temporal_lo: i32,
    /// The last frame read, relative to the one being rendered.
    pub temporal_hi: i32,
    /// Which alpha the effect's maths expects; unset lowers to premultiplied.
    pub alpha: LfxAlpha,
    /// Seeded, thread-unsafe, cancellable.
    pub flags: LfxTraitFlags,
    /// The working memory one megapixel of output costs.
    pub scratch_bytes_per_megapixel: u32,
}

impl Traits {
    /// The same declaration, read off the frozen struct field by field.
    ///
    /// A **short or zeroed block reads as the pessimistic case** and needs no
    /// help from here to do it: `lfx_traits` starts every enumeration at
    /// `UNSET = 0`, so a `memset` block and a block whose tail the host's
    /// header has but the plugin's did not are the same declaration, and
    /// [`crate::schema::traits_of`] lowers that declaration to heavy and full
    /// frame. This function therefore has no defaulting of its own to do.
    #[must_use]
    pub const fn from_abi(traits: &LfxTraits) -> Self {
        Self {
            cost: traits.cost,
            roi_kind: traits.roi_kind,
            roi_padding_px: traits.roi_padding_px,
            temporal_lo: traits.temporal_lo,
            temporal_hi: traits.temporal_hi,
            alpha: traits.alpha,
            flags: traits.flags,
            scratch_bytes_per_megapixel: traits.scratch_bytes_per_megapixel,
        }
    }
}

/// What the descriptor says about an effect, before `describe` has run.
///
/// The bundle's manifest states all of this too, and the host reads that first
/// so it can list an effect without opening the module. The manifest is the
/// cheap listing and this is the truth; a disagreement once the module is open
/// refuses the plugin - [`crate::manifest::agrees`] is that comparison, run at
/// the first describe (docs/impl/lfx.md §4.3).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Identity {
    /// Reverse-DNS, and stable for the plugin's life.
    pub id: String,
    /// What a person reads in the Add-effect menu.
    pub name: String,
    /// Who to blame, shown in the row's context menu.
    pub vendor: String,
    /// The major version.
    pub major: u32,
    /// The minor version.
    pub minor: u32,
    /// The patch version.
    pub patch: u32,
    /// The picture families claimed: the **first is the heading**, the rest are
    /// search keywords.
    pub categories: Vec<LfxCategory>,
    /// `None` is the pessimistic case, and means it.
    pub traits: Option<Traits>,
    /// The extensions without which this effect cannot work.
    pub required_extensions: Vec<String>,
}

/// One described plugin, entire: who it is, and what it declared.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PluginDescriptor {
    /// What the descriptor said.
    pub identity: Identity,
    /// The controls, in declaration order.
    pub params: Vec<Declaration>,
    /// The headings over them.
    pub groups: Vec<GroupRun>,
    /// What the sink declined, each a line in the scan report.
    pub report: Vec<LfxRejection>,
}

impl PluginDescriptor {
    /// Put the two halves together: what the descriptor said, and what
    /// `describe` pushed.
    #[must_use]
    pub fn new(identity: Identity, described: Described) -> Self {
        Self {
            identity,
            params: described.params,
            groups: described.groups,
            report: described.report,
        }
    }
}

/// What `describe` pushes its declarations into.
///
/// See the module header for the two kinds of `false`. A [`Describe`] is used
/// once: [`Describe::finish`] consumes it, because a sink that could be read
/// while the plugin was still pushing into it would be a half-described effect
/// somebody could catalogue.
#[derive(Clone, Debug, Default)]
pub struct Describe {
    params: Vec<Declaration>,
    groups: Vec<GroupRun>,
    report: Vec<LfxRejection>,
    /// The first structural fault, which ends the effect whatever is declared
    /// after it. The first rather than the last: it is the one that has not
    /// already been made worse by the faults following it.
    fault: Option<LfxRejection>,
    /// The headings opened and not yet closed, innermost last: `Some(index)`
    /// for one that opened a run in `groups`, `None` for one that was declined
    /// and is still waiting for its `group_end`.
    ///
    /// A stack rather than a count, because the two are only the same answer
    /// while every declined heading is the innermost one. A heading declined
    /// for its own label opens nothing, so a count would have the *next*
    /// heading's close swallowed instead of its own - and every row after that
    /// close would join a run the plugin never put it in, with no
    /// [`LfxRejection::GroupInsideGroup`] anywhere to say so.
    depth: Vec<Option<usize>>,
    /// Every row id minted so far, so a collision is caught as it happens
    /// rather than at the end. A map rather than a list: a plugin may declare
    /// as many rows as [`LFX_MAX_PARAMS`] allows and a linear scan per
    /// declaration would make the describe quadratic in what a stranger
    /// declares.
    minted: HashMap<ParamId, String>,
    /// Whether the declaration ceiling has already been recorded, so a describe
    /// loop that ran away puts one line in the report rather than one per
    /// declaration it goes on to make.
    full: bool,
    /// How many declarations have been **pushed**, accepted or not.
    ///
    /// Not the accepted count, which is what the panel costs. This is what the
    /// *sink* costs: every entry point files an owned string for a declaration
    /// it declines, so a loop of declarations nothing accepts grows the report
    /// exactly as a loop of good ones grows the panel, and a ceiling read off
    /// `params.len()` would never trip on it. Controls and headings share the
    /// budget, since they share the sink.
    pushed: u32,
}

impl Describe {
    /// A fresh sink.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Push one control. `true` if it was accepted.
    ///
    /// The checks, in the order a plugin meets them: an effect may push at
    /// most [`LFX_MAX_PARAMS`] declarations into the sink, accepted or not; the
    /// id must not be inside
    /// [`DERIVED_PREFIX`](crate::def::DERIVED_PREFIX), which is the host's own
    /// corner of the resolved bag; the unit must be one somebody decided
    /// on and not the diagonal's per cent; the kind's own numbers must make a
    /// control, which is one question asked in one place - every ceiling the
    /// frozen header declares, every range the panel could not draw, every
    /// whole number the resolved bag could not hold; and no row it mints may
    /// collide with one already minted.
    ///
    /// A unit declared on a kind that carries none is normalised rather than
    /// refused, and the normalisation is a line in the report: the header says
    /// an angle's unit *must* be `LFX_UNIT_DEGREES`, so an author whose
    /// declaration was quietly rewritten is owed the sentence saying so.
    pub fn declare(&mut self, declaration: Declaration) -> bool {
        if self.count_push() {
            return self.too_many();
        }

        // The host's own corner of the resolved bag, refused before anything is
        // minted from the id. `LfxDef::resolve_derived` writes `derived.frame`
        // into the same bag every route out of a definition reads, so a control
        // declared inside the prefix is a row whose value is overwritten before
        // the plugin reads it. Asked of the declaration's own id rather than of
        // the rows it mints, because every row a declaration mints is that id
        // with a suffix.
        if declaration.id.starts_with(crate::def::DERIVED_PREFIX) {
            return self.refuse(LfxRejection::ReservedParamId {
                id: declaration.id.clone(),
            });
        }

        // Exhaustive, with no `_` arm: a unit added to Lumit's own list has to
        // decide here whether a stranger's effect may declare it.
        let unusable = match declaration.unit {
            Unit::Unset => Some(LfxRejection::UnitUnset {
                id: declaration.id.clone(),
            }),
            Unit::PctDiag => Some(LfxRejection::UnitPctDiag {
                id: declaration.id.clone(),
            }),
            Unit::Raw | Unit::Percent | Unit::Px | Unit::Degrees | Unit::Seconds | Unit::Frames => {
                None
            }
        };
        if let Some(why) = unusable {
            return self.refuse(why);
        }

        if let Some(why) = undrawable(&declaration) {
            return self.line(why);
        }

        // Accepted, and one thing about it was still not taken at face value.
        // Pushed straight onto the report rather than through `decline`,
        // because the row is drawn: only its unit was not the plugin's to
        // choose.
        if schema::forced_unit(declaration.kind.tag())
            .is_some_and(|fixed| fixed != declaration.unit)
        {
            self.report.push(LfxRejection::UnitIgnoredForKind {
                id: declaration.id.clone(),
                declared: declaration.unit,
            });
        }

        // The other declared field this build has nowhere to put, and told the
        // same way for the same reason. `ParamSchema` carries no
        // non-animatable flag, so a row declared static is drawn keyframeable
        // whatever the plugin said; a kind that never animates here anyway
        // says nothing, since nothing was ignored.
        if declaration.is_static() && schema::animates(declaration.kind.tag()) {
            self.report.push(LfxRejection::StaticRowAnimatesAnyway {
                id: declaration.id.clone(),
            });
        }

        // Minted here rather than at the end, so that the `false` this sink
        // owes a plugin for a duplicate id (the ABI's own words) is answered on
        // the call that made the duplicate. `schema_of` checks the same thing
        // again, over the same minting, for a descriptor that never came
        // through a sink at all - a proto message, a test.
        let rows = schema::row_ids(&declaration);
        for row in &rows {
            if let Some(first) = self.minted.get(&ParamId::new(row)) {
                let first = first.clone();
                return self.refuse(LfxRejection::DuplicateParamId {
                    first,
                    second: row.clone(),
                });
            }
        }
        for row in rows {
            self.minted.insert(ParamId::new(&row), row);
        }

        self.params.push(declaration);
        true
    }

    /// Record a declaration of a kind this host does not admit. Always `false`.
    ///
    /// **No plugin can reach this**, and that is the point of naming it for
    /// what it is. The frozen sink has an entry point per kind it admits and
    /// there is no `declare_string` and no `declare_path`, so a v1 plugin
    /// cannot make either declaration through the ABI at all. The caller is
    /// whoever decodes a declaration that did not come through that sink - the
    /// broker, reading a proto message whose kind tag this version reserves
    /// (`LFX_PARAM_STRING`, `LFX_PARAM_PATH`) or does not know at all.
    ///
    /// Both reserved discriminants exist in the frozen enum from day one, so
    /// admitting them when the Viewer overlay and a text row land adds no
    /// variant and breaks no compiled plugin. Until then the refusal is by
    /// name, which is the contract saying *not yet* rather than a gap - and a
    /// tag that is neither is [`LfxRejection::UnknownParamKind`] rather than a
    /// path, so the report never calls a control something it is not.
    pub fn decline_kind(&mut self, kind: LfxParamKind, id: &str) -> bool {
        if self.count_push() {
            return self.too_many();
        }
        let why = match kind {
            LFX_PARAM_STRING => LfxRejection::NoTextRow { id: id.to_owned() },
            LFX_PARAM_PATH => LfxRejection::PathNeedsOverlay { id: id.to_owned() },
            declared => LfxRejection::UnknownParamKind {
                declared,
                id: id.to_owned(),
            },
        };
        self.line(why)
    }

    /// Open a heading over the rows declared until the matching
    /// [`Describe::group_end`]. `true` if it was accepted.
    ///
    /// A heading is a declaration pushed into the sink like any other, so it
    /// meets the same two ceilings: [`LFX_MAX_PARAMS`] declarations pushed, and
    /// [`LFX_MAX_STRING_BYTES`] for its id and its label. Whichever way it is
    /// declined it goes on the stack of open headings, so that it swallows its
    /// **own** `group_end` and the rows after that close keep whatever run they
    /// were in rather than leaving it.
    ///
    /// Nesting is asked of the stack rather than of the run currently open, so
    /// a heading opened inside a *declined* one is declined too. That is what
    /// the plugin declared, and the alternative would have the inner heading
    /// accepted, the outer one's close eaten on its behalf and the rows after
    /// it drawn under a twirl nobody put them in.
    pub fn group_begin(&mut self, id: &str, label: &str, flags: LfxParamFlags) -> bool {
        if self.count_push() {
            // Nothing is pushed onto the stack here: the effect is refused, and
            // a frame per call past the ceiling would be the growth this gate
            // exists to stop, arriving by the road the gate opened.
            return self.too_many();
        }
        if !self.depth.is_empty() {
            self.depth.push(None);
            return self.line(LfxRejection::GroupInsideGroup { id: id.to_owned() });
        }
        if let Some(bytes) = over_long(id).or_else(|| over_long(label)) {
            self.depth.push(None);
            return self.line(LfxRejection::StringTooLong {
                id: id.to_owned(),
                bytes,
            });
        }
        self.depth.push(Some(self.groups.len()));
        self.groups.push(GroupRun {
            id: id.to_owned(),
            label: label.to_owned(),
            hidden: flags & LFX_PARAM_FLAG_HIDDEN != 0,
            first: self.params.len(),
            len: 0,
        });
        true
    }

    /// Close the heading the last [`Describe::group_begin`] opened. `true` if
    /// there was one to close.
    ///
    /// A close is a push like any other and meets the same ceiling: a loop of
    /// nothing but `group_end` with none open would otherwise file a report
    /// line a call for ever.
    pub fn group_end(&mut self) -> bool {
        if self.count_push() {
            return self.too_many();
        }
        match self.depth.pop() {
            Some(Some(index)) => {
                self.close(index);
                true
            }
            // The heading this close belongs to was declined, so the close is
            // swallowed with it: there is no run to end and nothing to report.
            Some(None) => false,
            None => self.line(LfxRejection::GroupEndWithNothingOpen),
        }
    }

    /// What was declared, or the one fault that ends the effect.
    ///
    /// # Errors
    ///
    /// The first structural refusal the sink met - a duplicate row id, a unit
    /// nobody stated, a per cent of the diagonal. Everything else is in
    /// [`Described::report`] beside an effect that still loads.
    pub fn finish(mut self) -> Result<Described, LfxRejection> {
        while let Some(frame) = self.depth.pop() {
            // A heading that was declined opened no run and was already
            // reported when it was declined, so only an accepted one is left
            // to end and to name.
            if let Some(index) = frame {
                let id = self.groups.get(index).map(|group| group.id.clone());
                self.close(index);
                if let Some(id) = id {
                    self.report.push(LfxRejection::GroupLeftOpen { id });
                }
            }
        }
        if let Some(fault) = self.fault {
            return Err(fault);
        }
        // A heading over nothing is not drawn: every row it would have held was
        // declined, and an empty twirl is a control that opens onto nothing.
        self.groups.retain(|group| group.len > 0);
        Ok(Described {
            params: self.params,
            groups: self.groups,
            report: self.report,
        })
    }

    /// Count one push into the sink and answer whether it is past
    /// [`LFX_MAX_PARAMS`].
    ///
    /// Every entry point counts, and counts before it decides anything:
    /// a declaration the sink goes on to decline costs the report an owned
    /// string all the same, which is the growth the ceiling is there to stop.
    fn count_push(&mut self) -> bool {
        self.pushed = self.pushed.saturating_add(1);
        self.pushed > LFX_MAX_PARAMS
    }

    /// The answer every entry point gives once the ceiling is past: the fault,
    /// recorded once, and `false` for ever after.
    fn too_many(&mut self) -> bool {
        if self.full {
            return false;
        }
        self.full = true;
        let declared = self.pushed;
        self.refuse(LfxRejection::TooManyParams { declared })
    }

    /// Record a report line the caller worked out for itself, and answer
    /// `false` - this row, and nothing else.
    ///
    /// It is public for one reason, and the reason is the counted
    /// declarations. A dropdown's options, a file row's filters and a tone
    /// curve's points arrive as a count and a pointer, and the count is a
    /// stranger's `uint32_t`: whoever is holding the plugin must check it
    /// against the header's ceiling **before** it reads the array, because
    /// reading four billion pointers is not a mistake a later check can undo.
    /// The line it files then names the count the plugin declared rather than
    /// the number that was read, which is the honest one.
    ///
    /// A refusal that ends the effect is routed to the fault instead of the
    /// report, so this door cannot be used to file a structural fault as a
    /// line beside an effect that loads.
    pub fn decline(&mut self, why: LfxRejection) -> bool {
        if self.count_push() {
            return self.too_many();
        }
        if why.refuses_the_effect() {
            return self.refuse(why);
        }
        self.line(why)
    }

    /// Record a report line against a declaration whose push has already been
    /// counted, and answer `false`.
    ///
    /// The road every entry point takes once it has decided the declaration is
    /// a line rather than a row. [`Describe::decline`] is the same answer given
    /// to a caller *outside* this module, and counts the push on the way in,
    /// because a declaration the host could not read is a declaration the
    /// plugin pushed.
    fn line(&mut self, why: LfxRejection) -> bool {
        self.report.push(why);
        false
    }

    /// Record the fault that ends the effect and answer `false`. The first one
    /// is kept; the rest are report lines, so the page prints what happened
    /// after it rather than only the first sentence.
    fn refuse(&mut self, why: LfxRejection) -> bool {
        if self.fault.is_none() {
            self.fault = Some(why);
        } else {
            self.report.push(why);
        }
        false
    }

    /// Give the heading at `index` the run of declarations pushed since it
    /// opened.
    fn close(&mut self, index: usize) {
        let end = self.params.len();
        if let Some(group) = self.groups.get_mut(index) {
            group.len = end.saturating_sub(group.first);
        }
    }
}

/// The first reason this declaration is a row the panel cannot draw, or `None`
/// for one it can.
///
/// **Every ceiling the frozen header declares is asked here**, because the
/// header says so: "the host enforces the same numbers where a stranger's bytes
/// arrive, so the two sides agree by construction rather than by coincidence".
/// A count the ABI does not carry is not a truncation to make quietly - a
/// hundred thousand options would leak a hundred thousand strings for the
/// session and draw a dropdown that never ends.
///
/// Every answer here is a **report line**: a dropdown this build cannot draw is
/// a row lost rather than a structural fault, and the effect still loads with
/// that row keeping the default it was declared with.
fn undrawable(declaration: &Declaration) -> Option<LfxRejection> {
    let id = &declaration.id;
    let too_long = |text: &str| {
        over_long(text).map(|bytes| LfxRejection::StringTooLong {
            id: id.clone(),
            bytes,
        })
    };
    let count = |values: usize| u32::try_from(values).unwrap_or(u32::MAX);

    if let Some(why) = too_long(id).or_else(|| too_long(&declaration.label)) {
        return Some(why);
    }

    match &declaration.kind {
        Declared::Float { slider, hard, .. } => range_unusable(id, Some(slider.0), Some(slider.1))
            .or_else(|| range_unusable(id, hard.0, hard.1)),
        Declared::Slider { range, .. } => range_unusable(id, Some(range.0), Some(range.1)),
        Declared::Int {
            default,
            slider,
            hard,
        } => {
            // The ABI declares these `int64_t` and the resolved bag holds an
            // `i32`, so a legal declaration outside that range would reach
            // `process` as a number the plugin never wrote. Asked before the
            // ordering, which then compares two numbers that fit.
            let whole = [
                Some(*default),
                Some(slider.0),
                Some(slider.1),
                hard.0,
                hard.1,
            ];
            if whole
                .into_iter()
                .flatten()
                .any(|value| i32::try_from(value).is_err())
            {
                return Some(LfxRejection::WholeNumberOutOfRange { id: id.clone() });
            }
            #[allow(clippy::cast_precision_loss)]
            let as_real = |value: i64| value as f64;
            range_unusable(id, Some(as_real(slider.0)), Some(as_real(slider.1)))
                .or_else(|| range_unusable(id, hard.0.map(as_real), hard.1.map(as_real)))
        }
        Declared::Colour { range, .. } => range_unusable(id, Some(range.0), Some(range.1)),
        Declared::Choice {
            options,
            default,
            dividers_after,
        } => {
            if options.is_empty() {
                return Some(LfxRejection::ChoiceWithNoOptions { id: id.clone() });
            }
            if count(options.len()) > LFX_MAX_OPTIONS {
                return Some(LfxRejection::TooManyOptions {
                    id: id.clone(),
                    declared: count(options.len()),
                });
            }
            if count(dividers_after.len()) > LFX_MAX_DIVIDERS {
                return Some(LfxRejection::TooManyDividers {
                    id: id.clone(),
                    declared: count(dividers_after.len()),
                });
            }
            // The two numbers that have to agree with the list beside them. A
            // default past the end draws a dropdown with nothing selected and
            // hands `process` an option the plugin never offered; a divider
            // past the end draws a rule after nothing. LFX declares both where
            // OFX cannot say either, so the declaration is Lumit's to hold to
            // its own numbers.
            let offered = count(options.len());
            if *default >= offered {
                return Some(LfxRejection::ChoiceDefaultOutOfRange {
                    id: id.clone(),
                    declared: *default,
                    options: offered,
                });
            }
            if let Some(past) = dividers_after.iter().find(|after| **after >= offered) {
                return Some(LfxRejection::DividerPastTheOptions {
                    id: id.clone(),
                    declared: *past,
                    options: offered,
                });
            }
            options.iter().find_map(|option| too_long(option))
        }
        Declared::Curve { default } => {
            let points = count(default.len());
            (!(LFX_MIN_CURVE_POINTS..=LFX_MAX_CURVE_POINTS).contains(&points)).then(|| {
                LfxRejection::CurvePointsOutOfRange {
                    id: id.clone(),
                    declared: points,
                }
            })
        }
        Declared::File {
            filter,
            filter_name,
        } => {
            if count(filter.len()) > LFX_MAX_FILTERS {
                return Some(LfxRejection::TooManyFilters {
                    id: id.clone(),
                    declared: count(filter.len()),
                });
            }
            filter
                .iter()
                .find_map(|extension| too_long(extension))
                .or_else(|| too_long(filter_name))
        }
        Declared::Point2 { slider, .. } | Declared::Point3 { slider, .. } => {
            range_unusable(id, Some(slider.0), Some(slider.1))
        }
        // An angle is deliberately unbounded, a seed has no declared default at
        // all, a switch is two states and a button is none: none of the four
        // carries a number a range could be wrong about.
        Declared::Angle { .. } | Declared::Bool { .. } | Declared::Seed | Declared::Action => None,
    }
}

/// How many bytes a string occupies with its NUL, when that is past the ABI's
/// own ceiling.
///
/// Asked of every string a stranger declares, wherever it arrives: a row's id,
/// label, option text and file filter here, and the descriptor's own id, name
/// and vendor in [`crate::schema::schema_of`], which is where those three
/// arrive.
pub(crate) fn over_long(text: &str) -> Option<u32> {
    let bytes = u32::try_from(text.len().saturating_add(1)).unwrap_or(u32::MAX);
    (bytes > LFX_MAX_STRING_BYTES).then_some(bytes)
}

/// Whether a numeric range is one the panel can draw, either side open.
///
/// A **stated** end must be a number: an infinite hard minimum pins every value
/// to infinity, and a `NaN` end is a range with no ends at all. An **absent**
/// end is the infinity the resolve already reads it as
/// (`lumit-core`'s `hard_range`), so an open side is not a fault. And the low
/// end may not be above the high one: the clamp is `max(lo).min(hi)`, so a
/// transposed pair - the commonest copy-paste slip there is - pins the control
/// to `hi` for every stored value, every keyframe and every expression result,
/// with nothing anywhere saying why.
fn range_unusable(id: &str, lo: Option<f64>, hi: Option<f64>) -> Option<LfxRejection> {
    let stated_unusable = |end: Option<f64>| end.is_some_and(|value| !value.is_finite());
    let (low, high) = (lo.unwrap_or(f64::NEG_INFINITY), hi.unwrap_or(f64::INFINITY));
    (stated_unusable(lo) || stated_unusable(hi) || low > high).then(|| {
        LfxRejection::RangeUnusable {
            id: id.to_owned(),
            lo: low,
            hi: high,
        }
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A plain number row, so a test that is about something else has one to
    /// declare.
    fn float(id: &str) -> Declaration {
        Declaration {
            id: id.to_owned(),
            label: id.to_owned(),
            unit: Unit::Raw,
            flags: 0,
            kind: Declared::Float {
                default: 0.0,
                slider: (0.0, 1.0),
                hard: (None, None),
            },
        }
    }

    /// Every variant says which kind of no it is, and the two lists are what
    /// the module header claims they are: the structural ones end the effect,
    /// the report lines do not.
    ///
    /// The lists are written out rather than derived, and one of them is where
    /// a variant added by a later package belongs. The compiler stops such an
    /// author at [`LfxRejection::refuses_the_effect`]'s own exhaustive `match` -
    /// that is what makes the answer deliberate - and the comment beside it
    /// sends them here. *ponytail:* nothing checks that they came: Rust has no
    /// stable way to enumerate an enum's variants, and a macro that declared
    /// `LfxRejection` would buy that check at the cost of the doc comment
    /// every variant carries.
    #[test]
    fn a_refusal_is_either_a_report_line_or_the_end_of_the_effect() {
        for structural in [
            LfxRejection::VersionOutOfRange {
                major: 5_000,
                minor: 0,
                patch: 0,
            },
            LfxRejection::DuplicateParamId {
                first: "a".into(),
                second: "a".into(),
            },
            LfxRejection::ReservedParamId {
                id: "derived.frame".into(),
            },
            LfxRejection::UnitUnset { id: "a".into() },
            LfxRejection::UnitPctDiag { id: "a".into() },
            LfxRejection::TemporalWindowUnusable { lo: 1, hi: 2 },
            LfxRejection::TooManyParams { declared: 513 },
            LfxRejection::ManifestMismatch {
                id: "a".into(),
                field: "vendor",
                manifest: "Example".into(),
                code: "Somebody else".into(),
            },
            LfxRejection::PastCeiling {
                ceiling: crate::Ceiling::Categories,
                subject: "a plugin",
                given: 99,
            },
            LfxRejection::IdentityStringTooLong {
                field: "id".into(),
                bytes: 2_000,
            },
        ] {
            assert!(
                structural.refuses_the_effect(),
                "{structural} let the effect through"
            );
        }
        for line in [
            LfxRejection::PathNeedsOverlay { id: "a".into() },
            LfxRejection::NoTextRow { id: "a".into() },
            LfxRejection::UnknownParamKind {
                declared: 99,
                id: "a".into(),
            },
            LfxRejection::StringTooLong {
                id: "a".into(),
                bytes: 2_000,
            },
            LfxRejection::UnreadableDeclaration {
                kind: lumit_lfx_abi::LFX_PARAM_FLOAT,
                bytes: 4,
            },
            LfxRejection::TooManyOptions {
                id: "a".into(),
                declared: 1_000,
            },
            LfxRejection::TooManyDividers {
                id: "a".into(),
                declared: 1_000,
            },
            LfxRejection::TooManyFilters {
                id: "a".into(),
                declared: 1_000,
            },
            LfxRejection::RangeUnusable {
                id: "a".into(),
                lo: 1.0,
                hi: 0.0,
            },
            LfxRejection::WholeNumberOutOfRange { id: "a".into() },
            LfxRejection::UnitIgnoredForKind {
                id: "a".into(),
                declared: Unit::Seconds,
            },
            LfxRejection::WindowHeldToTheRing {
                wanted: 130,
                slots: 64,
            },
            LfxRejection::RingNarrowedByTheLedger {
                wanted: 32,
                granted: 3,
            },
            LfxRejection::StaticRowAnimatesAnyway { id: "a".into() },
            LfxRejection::ChoiceDefaultOutOfRange {
                id: "a".into(),
                declared: 7,
                options: 2,
            },
            LfxRejection::DividerPastTheOptions {
                id: "a".into(),
                declared: 7,
                options: 2,
            },
            LfxRejection::TooManyCategories { declared: 99 },
            LfxRejection::TooManyRequiredExtensions { declared: 99 },
            LfxRejection::DuplicateEffectId { id: "a".into() },
            LfxRejection::UnknownCategory { declared: 99 },
            LfxRejection::NoCategoryDeclared,
            LfxRejection::PaddingWithoutDistance,
            LfxRejection::ChoiceWithNoOptions { id: "a".into() },
            LfxRejection::CurvePointsOutOfRange {
                id: "a".into(),
                declared: 1,
            },
            LfxRejection::GroupInsideGroup { id: "a".into() },
            LfxRejection::GroupEndWithNothingOpen,
            LfxRejection::GroupLeftOpen { id: "a".into() },
        ] {
            assert!(!line.refuses_the_effect(), "{line} ended the effect");
        }
    }

    /// Unit is mandatory: a control that declared none is a refusal by name,
    /// and the effect does not load however good the rest of it is
    /// (docs/impl/lfx.md D4, §14 item 3).
    #[test]
    fn a_control_with_no_unit_refuses_the_effect() {
        let mut sink = Describe::new();
        assert!(sink.declare(float("before")));
        let mut unstated = float("radius");
        unstated.unit = Unit::Unset;
        assert!(!sink.declare(unstated), "an unstated unit was accepted");
        assert!(sink.declare(float("after")), "the plugin may carry on");

        assert_eq!(
            sink.finish(),
            Err(LfxRejection::UnitUnset {
                id: "radius".to_owned()
            })
        );
    }

    /// A per cent of the diagonal is what the ROI padding is declared in and
    /// what no control may be in - the refusal
    /// `no_parameter_is_a_per_cent_of_the_diagonal` gives a built-in, given to
    /// a stranger's effect at describe.
    #[test]
    fn a_per_cent_of_the_diagonal_refuses_the_effect() {
        let mut sink = Describe::new();
        let mut diagonal = float("reach");
        diagonal.unit = Unit::PctDiag;
        assert!(!sink.declare(diagonal));
        assert_eq!(
            sink.finish(),
            Err(LfxRejection::UnitPctDiag {
                id: "reach".to_owned()
            })
        );
    }

    /// Two rows on one [`ParamId`] is one control driving another, so the
    /// effect is refused rather than shipped ambiguous - and the collision is
    /// found across the spread as well as within it, since a point's `_x` is a
    /// row like any other.
    #[test]
    fn two_controls_on_one_param_id_refuse_the_effect() {
        let mut sink = Describe::new();
        assert!(sink.declare(Declaration {
            id: "centre".to_owned(),
            label: "Centre".to_owned(),
            unit: Unit::Px,
            flags: 0,
            kind: Declared::Point2 {
                default: (0.0, 0.0),
                slider: (0.0, 1.0),
            },
        }));
        assert!(
            !sink.declare(float("centre_x")),
            "a row collided with the point's x half and was accepted"
        );
        assert_eq!(
            sink.finish(),
            Err(LfxRejection::DuplicateParamId {
                first: "centre_x".to_owned(),
                second: "centre_x".to_owned(),
            })
        );
    }

    /// A row declared inside the host's own prefix is refused rather than
    /// silently overwritten (§4.2).
    ///
    /// `LfxDef::resolve_derived` pushes `derived.frame` into the same resolved
    /// bag `LfxDef::values_of` reads on every road out, and the built-ins push
    /// a dozen more `derived.` values of their own. Without this refusal a
    /// plugin declaring that id draws a panel row whose value is replaced by
    /// the comp's frame number before the plugin ever reads it - no line, no
    /// refusal, and a control that does nothing.
    ///
    /// The prefix is swept rather than the one name, since every one of them
    /// has the same ending; and a row that merely *mentions* it is a row like
    /// any other, because the rule is the prefix and not the word.
    #[test]
    fn a_row_inside_the_hosts_own_prefix_refuses_the_effect() {
        for reserved in ["derived.frame", "derived.px_scale", "derived."] {
            let mut sink = Describe::new();
            assert!(
                !sink.declare(float(reserved)),
                "{reserved} is the host's to write and was accepted"
            );
            assert_eq!(
                sink.finish(),
                Err(LfxRejection::ReservedParamId {
                    id: reserved.to_owned()
                })
            );
        }

        let mut sink = Describe::new();
        for ordinary in ["derived", "derivedness", "my.derived.frame"] {
            assert!(
                sink.declare(float(ordinary)),
                "{ordinary} is not inside the prefix and is a row like any other"
            );
        }
        assert!(sink.finish().is_ok());
    }

    /// The kinds this version reserves are a line in the report and nothing
    /// worse: the plugin loads, and the row keeps the default it was declared
    /// with for the effect's life (docs/12 §3.6, §14 item 3).
    ///
    /// Driven from the **tag**, because that is the only road either kind can
    /// arrive by: the frozen sink has thirteen declaration entry points and
    /// neither `declare_string` nor `declare_path` is one of them, so no v1
    /// plugin can push either declaration through the ABI. The caller is the
    /// broker, decoding a proto message whose kind tag this host does not
    /// admit - and a tag that is neither reserved kind is named as the unknown
    /// it is rather than printed as a path the author never declared.
    #[test]
    fn a_kind_this_version_reserves_is_a_report_line_and_the_plugin_still_loads() {
        let mut sink = Describe::new();
        assert!(sink.declare(float("amount")));
        assert!(!sink.decline_kind(LFX_PARAM_STRING, "caption"));
        assert!(!sink.decline_kind(LFX_PARAM_PATH, "outline"));
        // A tag from a header this build has never seen, and the tag a zeroed
        // record carries. Neither is a path.
        assert!(!sink.decline_kind(9_999, "whatever"));
        assert!(!sink.decline_kind(lumit_lfx_abi::LFX_PARAM_UNSET, "nothing"));
        assert!(sink.declare(float("softness")));

        let described = sink.finish().expect("a reserved kind does not end it");
        assert_eq!(described.params.len(), 2, "a reserved kind minted a row");
        assert_eq!(
            described.report,
            vec![
                LfxRejection::NoTextRow {
                    id: "caption".to_owned()
                },
                LfxRejection::PathNeedsOverlay {
                    id: "outline".to_owned()
                },
                LfxRejection::UnknownParamKind {
                    declared: 9_999,
                    id: "whatever".to_owned()
                },
                LfxRejection::UnknownParamKind {
                    declared: lumit_lfx_abi::LFX_PARAM_UNSET,
                    id: "nothing".to_owned()
                },
            ]
        );
    }

    /// A dropdown with nothing to choose and a tone curve that is not a curve
    /// are declined the same way: the row is not drawn, the effect still loads.
    #[test]
    fn a_control_whose_numbers_make_no_control_is_a_report_line() {
        let mut sink = Describe::new();
        assert!(!sink.declare(Declaration {
            id: "mode".to_owned(),
            label: "Mode".to_owned(),
            unit: Unit::Raw,
            flags: 0,
            kind: Declared::Choice {
                options: Vec::new(),
                default: 0,
                dividers_after: Vec::new(),
            },
        }));
        assert!(!sink.declare(Declaration {
            id: "shape".to_owned(),
            label: "Shape".to_owned(),
            unit: Unit::Raw,
            flags: 0,
            kind: Declared::Curve {
                default: vec![[0.0, 0.0]],
            },
        }));

        let described = sink.finish().expect("neither ends the effect");
        assert!(described.params.is_empty());
        assert_eq!(
            described.report,
            vec![
                LfxRejection::ChoiceWithNoOptions {
                    id: "mode".to_owned()
                },
                LfxRejection::CurvePointsOutOfRange {
                    id: "shape".to_owned(),
                    declared: 1,
                },
            ]
        );
    }

    /// A heading holds exactly the run declared inside it, and the rows outside
    /// belong to nothing.
    #[test]
    fn a_heading_holds_the_run_of_rows_declared_inside_it() {
        let mut sink = Describe::new();
        assert!(sink.declare(float("amount")));
        assert!(sink.group_begin("advanced", "Advanced", 0));
        assert!(sink.declare(float("threshold")));
        assert!(sink.declare(float("falloff")));
        assert!(sink.group_end());
        assert!(sink.declare(float("mix")));

        let described = sink.finish().expect("nothing structural happened");
        assert_eq!(
            described.groups,
            vec![GroupRun {
                id: "advanced".to_owned(),
                label: "Advanced".to_owned(),
                hidden: false,
                first: 1,
                len: 2,
            }]
        );
        assert!(described.report.is_empty());
    }

    /// Headings do not nest, because a nested run would split the outer one in
    /// two and draw its header twice. The inner one is declined, its rows join
    /// the heading already open, and its own close does not shut that heading
    /// out from under the rows that follow.
    #[test]
    fn a_heading_inside_a_heading_joins_the_one_already_open() {
        let mut sink = Describe::new();
        assert!(sink.group_begin("outer", "Outer", 0));
        assert!(sink.declare(float("first")));
        assert!(!sink.group_begin("inner", "Inner", 0));
        assert!(sink.declare(float("second")));
        assert!(!sink.group_end(), "the inner close was the declined one");
        assert!(sink.declare(float("third")));
        assert!(sink.group_end());
        assert!(sink.declare(float("outside")));

        let described = sink.finish().expect("nesting does not end the effect");
        assert_eq!(
            described.groups,
            vec![GroupRun {
                id: "outer".to_owned(),
                label: "Outer".to_owned(),
                hidden: false,
                first: 0,
                len: 3,
            }],
            "the outer heading did not keep all three rows"
        );
        assert_eq!(
            described.report,
            vec![LfxRejection::GroupInsideGroup {
                id: "inner".to_owned()
            }]
        );
    }

    /// A heading opened inside one that was **declined** is declined too, and
    /// the rows after the inner close belong to nothing.
    ///
    /// The nesting road and the label road are two different states, and a
    /// count of declined headings cannot tell them apart. A heading declined
    /// for its own label opens no run, so the next heading would be accepted,
    /// its close eaten on the declined one's behalf, and every row after that
    /// close drawn under a twirl the plugin never put it in - with no
    /// [`LfxRejection::GroupInsideGroup`] anywhere to say what happened. The
    /// stack answers what the plugin actually declared: both headings are
    /// inside something, so both are declined and all three rows are loose.
    #[test]
    fn a_heading_inside_one_declined_for_its_label_is_declined_too() {
        let long = "x".repeat(LFX_MAX_STRING_BYTES as usize);
        let mut sink = Describe::new();
        assert!(!sink.group_begin("outer", &long, 0));
        assert!(sink.declare(float("first")));
        assert!(
            !sink.group_begin("inner", "Inner", 0),
            "a heading inside a declined one was opened"
        );
        assert!(sink.declare(float("second")));
        assert!(!sink.group_end(), "the inner close was the declined one");
        assert!(sink.declare(float("third")));
        assert!(!sink.group_end(), "the outer close was the declined one");
        assert!(sink.declare(float("fourth")));

        let described = sink.finish().expect("neither heading ends the effect");
        assert!(
            described.groups.is_empty(),
            "a declined heading drew a run: {:?}",
            described.groups
        );
        assert_eq!(described.params.len(), 4, "a row went missing");
        assert_eq!(
            described.report,
            vec![
                LfxRejection::StringTooLong {
                    id: "outer".to_owned(),
                    bytes: LFX_MAX_STRING_BYTES + 1,
                },
                LfxRejection::GroupInsideGroup {
                    id: "inner".to_owned()
                },
            ]
        );
    }

    /// A row declared static on a kind this build animates is a line in the
    /// report: the flag is read, and this build has nowhere to put it.
    ///
    /// `ParamSchema` carries no non-animatable field, so the row is drawn
    /// keyframeable whatever the declaration said - and a mandatory field whose
    /// value is thrown away without a word is the fault left to be found
    /// somewhere later. A tone curve, a file choice and a button are static in
    /// Lumit already, which is the header's own example of the flag, so on
    /// those three nothing was ignored and nothing is said.
    #[test]
    fn a_static_row_this_build_animates_is_a_report_line() {
        let fixed = |id: &str, kind: Declared| Declaration {
            id: id.to_owned(),
            label: id.to_owned(),
            unit: Unit::Raw,
            flags: LFX_PARAM_FLAG_STATIC,
            kind,
        };
        let mut sink = Describe::new();
        let mut pinned = float("strength");
        pinned.flags = LFX_PARAM_FLAG_STATIC | LFX_PARAM_FLAG_HIDDEN;
        assert!(sink.declare(pinned), "a static row lost its control");
        assert!(sink.declare(fixed(
            "shape",
            Declared::Curve {
                default: vec![[0.0, 0.0], [1.0, 1.0]],
            }
        )));
        assert!(sink.declare(fixed(
            "table",
            Declared::File {
                filter: vec!["cube".to_owned()],
                filter_name: "LUTs".to_owned(),
            }
        )));
        assert!(sink.declare(fixed("analyse", Declared::Action)));
        // The ordinary row, which declared nothing and is owed nothing.
        assert!(sink.declare(float("mix")));

        let described = sink.finish().expect("the flag does not end the effect");
        assert_eq!(described.params.len(), 5, "a static row left the panel");
        assert_eq!(
            described.report,
            vec![LfxRejection::StaticRowAnimatesAnyway {
                id: "strength".to_owned()
            }],
            "a kind that never animates here was named, or one that does was not"
        );
    }

    /// A dropdown whose own numbers do not agree with its option list is a line
    /// in the report and the row is not drawn: a default that selects an option
    /// it has not got, and a rule drawn after one.
    ///
    /// LFX declares both where OFX can say neither, so the declaration is
    /// Lumit's to hold to its own numbers. Declined whole rather than trimmed,
    /// which is the answer the header already gives for this same list: "a
    /// ceiling here declines the whole declaration rather than trimming it".
    #[test]
    fn a_dropdown_that_names_an_option_it_has_not_got_is_a_report_line() {
        let dropdown = |default: u32, dividers_after: Vec<u32>| Declaration {
            id: "mode".to_owned(),
            label: "Mode".to_owned(),
            unit: Unit::Raw,
            flags: 0,
            kind: Declared::Choice {
                options: vec!["Box".to_owned(), "Gaussian".to_owned()],
                default,
                dividers_after,
            },
        };
        for (declaration, expected) in [
            (
                dropdown(7, Vec::new()),
                LfxRejection::ChoiceDefaultOutOfRange {
                    id: "mode".to_owned(),
                    declared: 7,
                    options: 2,
                },
            ),
            (
                dropdown(0, vec![4_000]),
                LfxRejection::DividerPastTheOptions {
                    id: "mode".to_owned(),
                    declared: 4_000,
                    options: 2,
                },
            ),
            // The index *at* the end names no option either: there is no
            // third option for a rule to follow.
            (
                dropdown(0, vec![2]),
                LfxRejection::DividerPastTheOptions {
                    id: "mode".to_owned(),
                    declared: 2,
                    options: 2,
                },
            ),
        ] {
            let mut sink = Describe::new();
            assert!(!sink.declare(declaration), "{expected} was accepted");
            let described = sink.finish().expect("a dropdown does not end the effect");
            assert!(described.params.is_empty(), "{expected} minted a row");
            assert_eq!(described.report, vec![expected]);
        }

        // The last option and the rule before it are both declarable: the
        // refusal is for what goes past the list, not for reaching its end.
        let mut sink = Describe::new();
        assert!(sink.declare(dropdown(1, vec![0])));
        assert!(sink.finish().expect("nothing structural").report.is_empty());
    }

    /// A heading left open ends where the declarations do, and a heading over
    /// nothing is dropped rather than drawn empty. Both are report lines.
    #[test]
    fn a_heading_left_open_ends_where_the_declarations_do() {
        let mut sink = Describe::new();
        assert!(sink.group_begin("empty", "Empty", 0));
        assert!(sink.group_end());
        assert!(!sink.group_end(), "there was nothing left to close");
        assert!(sink.group_begin("tail", "Tail", LFX_PARAM_FLAG_HIDDEN));
        assert!(sink.declare(float("last")));

        let described = sink.finish().expect("nothing structural happened");
        assert_eq!(
            described.groups,
            vec![GroupRun {
                id: "tail".to_owned(),
                label: "Tail".to_owned(),
                hidden: true,
                first: 0,
                len: 1,
            }]
        );
        assert_eq!(
            described.report,
            vec![
                LfxRejection::GroupEndWithNothingOpen,
                LfxRejection::GroupLeftOpen {
                    id: "tail".to_owned()
                },
            ]
        );
    }

    /// A range the panel cannot draw is a line in the report and the row is
    /// not drawn: an end that is not a number, and a pair the wrong way round.
    ///
    /// The transposed pair is why this is worth a refusal rather than a
    /// lowering. `hard_range` hands the resolve `(Some(100), Some(0))` and the
    /// clamp is `max(100).min(0)`, so every stored value, every keyframe and
    /// every expression result on that row resolves to nought for the effect's
    /// life - the control is dead and nothing anywhere says why.
    #[test]
    fn a_range_the_panel_cannot_draw_is_a_report_line() {
        let transposed = |kind: Declared| Declaration {
            id: "amount".to_owned(),
            label: "Amount".to_owned(),
            unit: Unit::Raw,
            flags: 0,
            kind,
        };
        let cases = [
            Declared::Float {
                default: 0.0,
                slider: (100.0, 0.0),
                hard: (None, None),
            },
            Declared::Float {
                default: 0.0,
                slider: (0.0, 1.0),
                hard: (Some(1.0), Some(-1.0)),
            },
            Declared::Float {
                default: 0.0,
                slider: (0.0, f64::NAN),
                hard: (None, None),
            },
            // A *stated* bound must be a number: an infinite hard minimum pins
            // every value to infinity.
            Declared::Float {
                default: 0.0,
                slider: (0.0, 1.0),
                hard: (Some(f64::INFINITY), None),
            },
            Declared::Slider {
                default: 0.0,
                range: (1.0, 0.0),
                log: false,
            },
            Declared::Int {
                default: 0,
                slider: (10, -10),
                hard: (None, None),
            },
            Declared::Colour {
                default: [0.0; 4],
                range: (1.0, 0.0),
            },
            Declared::Point2 {
                default: (0.0, 0.0),
                slider: (1.0, -1.0),
            },
        ];
        for kind in cases {
            let mut sink = Describe::new();
            assert!(
                !sink.declare(transposed(kind.clone())),
                "{kind:?} was accepted"
            );
            let described = sink.finish().expect("a range does not end the effect");
            assert!(described.params.is_empty(), "{kind:?} minted a row");
            assert!(
                matches!(
                    described.report.first(),
                    Some(LfxRejection::RangeUnusable { id, .. }) if id == "amount"
                ),
                "{kind:?} was declined as {:?}",
                described.report
            );
        }

        // An *open* side is not a fault: that is what an unbounded Float is,
        // and the resolve already reads an absent bound as the infinity.
        let mut sink = Describe::new();
        assert!(sink.declare(transposed(Declared::Float {
            default: 0.0,
            slider: (0.0, 1.0),
            hard: (Some(0.0), None),
        })));
        assert!(sink.finish().expect("nothing structural").report.is_empty());
    }

    /// A whole number outside the range a stored value holds is declined
    /// rather than wrapped.
    ///
    /// The ABI declares an `INT` row's default and bounds as `int64_t`, and
    /// the resolved bag holds an `i32` that the backfill reaches through a
    /// truncating cast. A legal `default_value` of three thousand million would
    /// otherwise reach `process` as minus one and a bit thousand million - a
    /// number the plugin never declared, with no line and no refusal.
    #[test]
    fn a_whole_number_outside_the_bag_is_a_report_line() {
        let whole = |kind: Declared| Declaration {
            id: "steps".to_owned(),
            label: "Steps".to_owned(),
            unit: Unit::Raw,
            flags: 0,
            kind,
        };
        for kind in [
            Declared::Int {
                default: 3_000_000_000,
                slider: (0, 10),
                hard: (None, None),
            },
            Declared::Int {
                default: 0,
                slider: (0, i64::from(i32::MAX) + 1),
                hard: (None, None),
            },
            Declared::Int {
                default: 0,
                slider: (0, 10),
                hard: (Some(i64::from(i32::MIN) - 1), None),
            },
        ] {
            let mut sink = Describe::new();
            assert!(!sink.declare(whole(kind.clone())), "{kind:?} was accepted");
            assert_eq!(
                sink.finish().expect("it does not end the effect").report,
                vec![LfxRejection::WholeNumberOutOfRange {
                    id: "steps".to_owned()
                }]
            );
        }

        // The extremes themselves are declarable: the refusal is for what falls
        // outside them, not for reaching them.
        let mut sink = Describe::new();
        assert!(sink.declare(whole(Declared::Int {
            default: 0,
            slider: (i64::from(i32::MIN), i64::from(i32::MAX)),
            hard: (Some(i64::from(i32::MIN)), Some(i64::from(i32::MAX))),
        })));
        assert!(sink.finish().expect("nothing structural").report.is_empty());
    }

    /// Every ceiling the frozen header declares is one the host asks, because
    /// the header says the host enforces the same numbers where a stranger's
    /// bytes arrive. Each costs one row and no more.
    #[test]
    fn a_declaration_past_the_abis_ceilings_is_a_report_line() {
        let options = |count: usize| Declared::Choice {
            options: vec!["Mode".to_owned(); count],
            default: 0,
            dividers_after: Vec::new(),
        };
        let long = "x".repeat(LFX_MAX_STRING_BYTES as usize);
        let cases: Vec<(Declaration, LfxRejection)> = vec![
            (
                Declaration {
                    id: "mode".to_owned(),
                    label: "Mode".to_owned(),
                    unit: Unit::Raw,
                    flags: 0,
                    kind: options(LFX_MAX_OPTIONS as usize + 1),
                },
                LfxRejection::TooManyOptions {
                    id: "mode".to_owned(),
                    declared: LFX_MAX_OPTIONS + 1,
                },
            ),
            (
                Declaration {
                    id: "mode".to_owned(),
                    label: "Mode".to_owned(),
                    unit: Unit::Raw,
                    flags: 0,
                    kind: Declared::Choice {
                        options: vec!["Mode".to_owned()],
                        default: 0,
                        dividers_after: vec![0; LFX_MAX_DIVIDERS as usize + 1],
                    },
                },
                LfxRejection::TooManyDividers {
                    id: "mode".to_owned(),
                    declared: LFX_MAX_DIVIDERS + 1,
                },
            ),
            (
                Declaration {
                    id: "table".to_owned(),
                    label: "Table".to_owned(),
                    unit: Unit::Raw,
                    flags: 0,
                    kind: Declared::File {
                        filter: vec!["cube".to_owned(); LFX_MAX_FILTERS as usize + 1],
                        filter_name: "LUTs".to_owned(),
                    },
                },
                LfxRejection::TooManyFilters {
                    id: "table".to_owned(),
                    declared: LFX_MAX_FILTERS + 1,
                },
            ),
            (
                Declaration {
                    id: "amount".to_owned(),
                    label: long.clone(),
                    unit: Unit::Raw,
                    flags: 0,
                    kind: Declared::Float {
                        default: 0.0,
                        slider: (0.0, 1.0),
                        hard: (None, None),
                    },
                },
                LfxRejection::StringTooLong {
                    id: "amount".to_owned(),
                    bytes: LFX_MAX_STRING_BYTES + 1,
                },
            ),
            (
                Declaration {
                    id: "mode".to_owned(),
                    label: "Mode".to_owned(),
                    unit: Unit::Raw,
                    flags: 0,
                    kind: Declared::Choice {
                        options: vec![long.clone()],
                        default: 0,
                        dividers_after: Vec::new(),
                    },
                },
                LfxRejection::StringTooLong {
                    id: "mode".to_owned(),
                    bytes: LFX_MAX_STRING_BYTES + 1,
                },
            ),
        ];
        for (declaration, expected) in cases {
            let mut sink = Describe::new();
            assert!(!sink.declare(declaration), "{expected} was accepted");
            let described = sink.finish().expect("a ceiling does not end the effect");
            assert!(described.params.is_empty(), "{expected} minted a row");
            assert_eq!(described.report, vec![expected]);
        }

        // A heading is a declaration pushed into the sink like any other, so
        // its own strings meet the same ceiling - and a declined heading
        // swallows its `group_end`, so the rows after it stay where they were.
        let mut sink = Describe::new();
        assert!(!sink.group_begin("advanced", &long, 0));
        assert!(sink.declare(float("amount")));
        assert!(
            !sink.group_end(),
            "the declined heading's close was swallowed"
        );
        let described = sink.finish().expect("a heading's label does not end it");
        assert!(described.groups.is_empty());
        assert_eq!(
            described.report,
            vec![LfxRejection::StringTooLong {
                id: "advanced".to_owned(),
                bytes: LFX_MAX_STRING_BYTES + 1,
            }]
        );

        // A dropdown right up against the ceiling is drawn: the refusal is for
        // what goes past it.
        let mut sink = Describe::new();
        assert!(sink.declare(Declaration {
            id: "mode".to_owned(),
            label: "Mode".to_owned(),
            unit: Unit::Raw,
            flags: 0,
            kind: options(LFX_MAX_OPTIONS as usize),
        }));
        assert!(sink.finish().expect("nothing structural").report.is_empty());
    }

    /// More declarations than the ABI carries refuses the **effect**, and says
    /// so once however long the describe loop goes on.
    ///
    /// A row lost is a control lost; a panel of half a million rows is a plugin
    /// this host cannot represent, and the rows it mints are leaked for the
    /// session. In process there is no watchdog to stop it, so the ceiling is
    /// the only thing that does.
    #[test]
    fn more_declarations_than_the_abi_carries_refuse_the_effect() {
        let mut sink = Describe::new();
        for index in 0..LFX_MAX_PARAMS {
            assert!(sink.declare(float(&format!("row_{index}"))), "row {index}");
        }
        for index in 0..8_u32 {
            assert!(
                !sink.declare(float(&format!("over_{index}"))),
                "row {index} past the ceiling was accepted"
            );
        }
        let refused = sink.finish().expect_err("the effect was not refused");
        assert_eq!(
            refused,
            LfxRejection::TooManyParams {
                declared: LFX_MAX_PARAMS + 1
            },
            "the ceiling was recorded more than once, or not at all"
        );

        // A heading is a declaration too - both halves of one, since a close is
        // a call into the sink like an open - and the two share one budget with
        // the rows, because they share the sink whose growth is being bounded.
        let mut headings = Describe::new();
        for index in 0..LFX_MAX_PARAMS / 2 {
            assert!(headings.group_begin(&format!("run_{index}"), "Run", 0));
            assert!(headings.group_end());
        }
        assert!(!headings.group_begin("over", "Over", 0));
        assert_eq!(
            headings.finish().expect_err("the effect was not refused"),
            LfxRejection::TooManyParams {
                declared: LFX_MAX_PARAMS + 1
            }
        );
    }

    /// The ceiling counts declarations **pushed**, not rows accepted, which is
    /// the whole of what it is for.
    ///
    /// Every road into the sink files an owned string for a declaration it
    /// declines - an id cloned from the plugin's, up to the ABI's own string
    /// ceiling - so a describe loop of declarations nothing accepts grows the
    /// report exactly as a loop of good ones grows the panel. A gate reading
    /// `params.len()` would never trip on it, and in process (the local host)
    /// there is no watchdog behind it. Driven down each of the six roads in
    /// turn, since each is one a plugin, a decoded message or the host holding
    /// the plugin can take.
    #[test]
    fn a_describe_loop_of_declined_declarations_meets_the_same_ceiling() {
        /// One push into the sink, driven the same way whichever road it is.
        type Road = fn(&mut Describe, u32) -> bool;
        let loops: Vec<Road> = vec![
            // A declaration the sink declines: nothing reaches `params`.
            |sink, index| {
                sink.declare(Declaration {
                    id: format!("mode_{index}"),
                    label: "Mode".to_owned(),
                    unit: Unit::Raw,
                    flags: 0,
                    kind: Declared::Choice {
                        options: Vec::new(),
                        default: 0,
                        dividers_after: Vec::new(),
                    },
                })
            },
            // A declaration the sink refuses structurally: nothing reaches
            // `params` there either, and every fault after the first is a line.
            |sink, index| {
                let mut unstated = float(&format!("row_{index}"));
                unstated.unit = Unit::Unset;
                sink.declare(unstated)
            },
            // A kind this version reserves, arriving off a decoded message.
            |sink, index| sink.decline_kind(LFX_PARAM_STRING, &format!("text_{index}")),
            // A heading opened inside one already open, for ever. The first
            // call opens a real one; every one after it is declined.
            |sink, index| sink.group_begin(&format!("heading_{index}"), "Heading", 0),
            // A close with nothing open, for ever.
            |sink, _| sink.group_end(),
            // A declaration whoever is holding the plugin could not read, which
            // is the public road `decline` exists for and counts on the way in.
            |sink, _| {
                sink.decline(LfxRejection::UnreadableDeclaration {
                    kind: LFX_PARAM_FLOAT,
                    bytes: 0,
                })
            },
        ];
        const NESTING: usize = 3;
        for (road, push) in loops.into_iter().enumerate() {
            let mut sink = Describe::new();
            for index in 0..LFX_MAX_PARAMS + 64 {
                let accepted = push(&mut sink, index);
                assert!(
                    !accepted || (road == NESTING && index == 0),
                    "road {road} accepted a declaration at {index}"
                );
            }

            // Read off the sink's own fields, which this module can see: what
            // the ceiling has to bound is the report, and `finish` drops it
            // along with everything else once the effect is refused.
            assert_eq!(
                sink.pushed,
                LFX_MAX_PARAMS + 64,
                "road {road} did not count every push"
            );
            assert!(
                sink.report.len() <= LFX_MAX_PARAMS as usize,
                "road {road} filed {} lines, which is past the ceiling",
                sink.report.len()
            );
            assert!(
                sink.params.is_empty() && sink.groups.len() <= 1,
                "road {road} minted something after all"
            );
            assert!(
                sink.depth.len() <= LFX_MAX_PARAMS as usize,
                "road {road} stacked {} open headings, which is past the ceiling",
                sink.depth.len()
            );

            // And the effect is refused. `TooManyParams` for the roads whose
            // declarations are only lines; for the road whose declarations are
            // structural it is the **first** of those, which is the one that
            // has not already been made worse by the faults after it - either
            // way the plugin is not catalogued and the loop is over.
            let refused = sink.finish().expect_err("the loop was not refused");
            assert!(
                refused.refuses_the_effect(),
                "road {road} was refused with a report line: {refused}"
            );
            if road != 1 {
                assert_eq!(
                    refused,
                    LfxRejection::TooManyParams {
                        declared: LFX_MAX_PARAMS + 1
                    },
                    "road {road} named the wrong ceiling"
                );
            }
        }
    }

    /// The three fields the ABI edge has to *decide* rather than copy are
    /// decided here, once, so that the in-process host and the broker cannot
    /// read one frozen struct two different ways.
    #[test]
    fn a_declaration_crosses_the_numbers_it_was_written_with() {
        let float = LfxFloatParam {
            struct_size: 0,
            unit: lumit_lfx_abi::LFX_UNIT_RAW,
            flags: 0,
            bounds: LFX_BOUND_MAX,
            id: std::ptr::null(),
            label: std::ptr::null(),
            default_value: 0.5,
            slider_min: 0.0,
            slider_max: 1.0,
            // Filled in, and **not** claimed: `bounds` says only the high side
            // is meant, so the low one is open rather than nought.
            hard_min: -7.0,
            hard_max: 2.0,
        };
        assert_eq!(
            Declared::float_from_abi(&float),
            Declared::Float {
                default: 0.5,
                slider: (0.0, 1.0),
                hard: (None, Some(2.0)),
            }
        );

        let int = LfxIntParam {
            struct_size: 0,
            unit: lumit_lfx_abi::LFX_UNIT_RAW,
            flags: 0,
            bounds: LFX_BOUND_MIN | LFX_BOUND_MAX,
            id: std::ptr::null(),
            label: std::ptr::null(),
            default_value: 3,
            slider_min: 0,
            slider_max: 10,
            hard_min: -1,
            hard_max: 99,
        };
        assert_eq!(
            Declared::int_from_abi(&int),
            Declared::Int {
                default: 3,
                slider: (0, 10),
                hard: (Some(-1), Some(99)),
            }
        );
        assert_eq!(
            Declared::int_from_abi(&LfxIntParam {
                bounds: lumit_lfx_abi::LFX_BOUND_NONE,
                ..int
            }),
            Declared::Int {
                default: 3,
                slider: (0, 10),
                hard: (None, None),
            },
            "a bound the mask does not claim was read anyway"
        );

        // Non-zero, and not "equal to one": a C author writing 2 meant yes.
        let slider = LfxSliderParam {
            struct_size: 0,
            unit: lumit_lfx_abi::LFX_UNIT_RAW,
            flags: 0,
            log: 2,
            id: std::ptr::null(),
            label: std::ptr::null(),
            default_value: 1_000.0,
            range_min: 20.0,
            range_max: 20_000.0,
        };
        assert_eq!(
            Declared::slider_from_abi(&slider),
            Declared::Slider {
                default: 1_000.0,
                range: (20.0, 20_000.0),
                log: true,
            }
        );
        assert_eq!(
            Declared::slider_from_abi(&LfxSliderParam { log: 0, ..slider }),
            Declared::Slider {
                default: 1_000.0,
                range: (20.0, 20_000.0),
                log: false,
            }
        );

        let switch = LfxBoolParam {
            struct_size: 0,
            unit: lumit_lfx_abi::LFX_UNIT_RAW,
            flags: 0,
            default_value: 2,
            id: std::ptr::null(),
            label: std::ptr::null(),
        };
        assert_eq!(
            Declared::bool_from_abi(&switch),
            Declared::Bool { default: true },
            "a truthy number that is not one was read as off"
        );
        assert_eq!(
            Declared::bool_from_abi(&LfxBoolParam {
                default_value: 0,
                ..switch
            }),
            Declared::Bool { default: false }
        );
    }

    /// A zeroed trait block crosses field for field, with no defaulting on the
    /// way: what makes it the pessimistic case is the lowering, and the
    /// lowering is somewhere a test can look at it.
    #[test]
    fn a_trait_block_crosses_the_numbers_it_was_written_with() {
        assert_eq!(Traits::from_abi(&LfxTraits::default()), Traits::default());

        let declared = LfxTraits {
            struct_size: 0,
            cost: lumit_lfx_abi::LFX_COST_CHEAP,
            roi_kind: lumit_lfx_abi::LFX_ROI_PADDED,
            roi_padding_px: 12.5,
            temporal_lo: -2,
            temporal_hi: 1,
            alpha: lumit_lfx_abi::LFX_ALPHA_STRAIGHT,
            flags: lumit_lfx_abi::LFX_TRAIT_SEEDED,
            scratch_bytes_per_megapixel: 4_096,
        };
        let mirrored = Traits::from_abi(&declared);
        assert_eq!(mirrored.cost, lumit_lfx_abi::LFX_COST_CHEAP);
        assert_eq!(mirrored.roi_padding_px, 12.5);
        assert_eq!((mirrored.temporal_lo, mirrored.temporal_hi), (-2, 1));
        assert_eq!(mirrored.alpha, lumit_lfx_abi::LFX_ALPHA_STRAIGHT);
        assert_eq!(mirrored.scratch_bytes_per_megapixel, 4_096);
    }
}
