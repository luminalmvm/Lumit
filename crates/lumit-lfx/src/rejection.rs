//! Why an LFX plugin is not an effect here, or is one control short of what it
//! declared.
//!
//! # In plain terms
//!
//! A plugin can be wrong in two different ways, and they get two different
//! answers. A control this build cannot draw - a text row, a bezier path, a
//! category nobody recognises, a dropdown with more options than the ABI
//! carries, a range whose ends are the wrong way round - is a **line in the
//! scan report**, and the plugin still loads keeping its own declared default
//! for that parameter (docs/12 §3.6). A *structural* fault is a refusal: two
//! controls that would drive each other, a unit nobody stated, a version number
//! the frame key cannot tell apart from another one, a window of neighbouring
//! frames nobody can honour, more declarations pushed into the sink than the
//! ABI admits, a plugin whose own name is longer than the ABI carries, a
//! declaration past a ceiling the frozen header put in writing.
//!
//! Both kinds are named here, in one enumeration, because both are sentences
//! the Addons page prints rather than silences, and because a validator that
//! can ask for one by name can ask for the other the same way. Which kind a
//! variant is is [`LfxRejection::refuses_the_effect`]'s answer - written once,
//! as an exhaustive match, rather than read off whichever list a reader
//! remembers it being on.

use lumit_core::fx::Unit;
use lumit_lfx_abi::{
    LFX_MAX_CATEGORIES, LFX_MAX_CURVE_POINTS, LFX_MAX_DIVIDERS, LFX_MAX_EFFECTS_PER_BUNDLE,
    LFX_MAX_FILTERS, LFX_MAX_OPTIONS, LFX_MAX_PARAMS, LFX_MAX_REQUIRED_EXTENSIONS,
    LFX_MAX_STRING_BYTES,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Every `&'static str` a refusal may carry, in one place.
///
/// Two of [`LfxRejection`]'s fields are words **this crate** wrote rather than
/// anything a plugin declared - the subject a ceiling was measured against, and
/// the manifest field that disagreed - and they are `&'static str` for the
/// reason they are not `String`: a refusal about a string nobody has checked
/// yet must not quote that string, so the word has to come from a closed set.
///
/// The set is here because a refusal now crosses a pipe, and a `&'static str`
/// cannot be deserialised into one. [`interned`] reads a word back by finding
/// it in this list, which is a stronger check than a `String` field would be:
/// a word off the wire that is not one of ours is a message this build cannot
/// read, and the conversation ends rather than the sentence being printed.
pub const INTERNED: [&str; 26] = [
    // The subjects of [`LfxRejection::PastCeiling`], which are [`SUBJECTS`] -
    // held to it by `every_ceiling_subject_is_a_word_a_refusal_can_carry`.
    subject::BUNDLE,
    subject::MANIFEST,
    subject::MODULE,
    subject::PLUGIN,
    subject::PLUGIN_ID,
    subject::PLUGIN_NAME,
    subject::PLUGIN_VENDOR,
    subject::REQUIRED_EXTENSION,
    subject::PANEL,
    subject::PLUGIN_REPORT,
    subject::CONTROL_ID,
    subject::CONTROL_LABEL,
    subject::DROPDOWN,
    subject::DROPDOWN_OPTION,
    subject::FILE_CONTROL,
    subject::FILE_FILTER,
    subject::TONE_CURVE,
    subject::HEADING_ID,
    subject::HEADING_LABEL,
    // The fields of [`LfxRejection::ManifestMismatch`], which are
    // `crate::manifest::COMPARED_FIELDS` - held to it by
    // `every_compared_field_is_a_word_a_refusal_can_carry`.
    "id",
    "name",
    "vendor",
    "version",
    "categories",
    "ABI version",
    "required extensions",
];

/// Every subject a [`LfxRejection::PastCeiling`] may name, in one place.
///
/// The twin of `crate::manifest::COMPARED_FIELDS`, and it exists for the same
/// reason: a word written as a literal at its call site is a word nothing
/// sweeps, and a subject that is not in [`INTERNED`] serialises in the broker
/// and fails to deserialise in the host - which ends the conversation, strikes
/// the plugin, and does it again on the deterministic retry until the bundle is
/// disabled for a report line nobody could read. So the call sites name a
/// constant from [`subject`] and this list holds every one of them, swept
/// against [`INTERNED`] by
/// `every_ceiling_subject_is_a_word_a_refusal_can_carry`.
pub const SUBJECTS: [Word; 19] = [
    subject::BUNDLE,
    subject::MANIFEST,
    subject::MODULE,
    subject::PLUGIN,
    subject::PLUGIN_ID,
    subject::PLUGIN_NAME,
    subject::PLUGIN_VENDOR,
    subject::REQUIRED_EXTENSION,
    subject::PANEL,
    subject::PLUGIN_REPORT,
    subject::CONTROL_ID,
    subject::CONTROL_LABEL,
    subject::DROPDOWN,
    subject::DROPDOWN_OPTION,
    subject::FILE_CONTROL,
    subject::FILE_FILTER,
    subject::TONE_CURVE,
    subject::HEADING_ID,
    subject::HEADING_LABEL,
];

/// What a ceiling was measured against, as the word the sentence prints.
///
/// One constant per subject rather than a literal at each call site, so that
/// [`SUBJECTS`] can be swept against [`INTERNED`] and a subject added for a new
/// measurement has somewhere to be added *to*.
///
/// *ponytail:* the [`Word`] doc's own fuller answer - a typed `Subject`
/// enumeration - would make an unreserved word unwritable rather than merely
/// unsweepable. This is the shape that holds the line until then.
pub mod subject {
    use super::Word;

    /// A whole bundle, counted in the effects it holds.
    pub const BUNDLE: Word = "the bundle";
    /// The bundle's own listing, counted in the entries it declares.
    pub const MANIFEST: Word = "the manifest";
    /// The module the listing describes, counted the same way.
    pub const MODULE: Word = "the module";
    /// One plugin, counted in the categories or the extensions it claims.
    pub const PLUGIN: Word = "a plugin";
    /// A plugin's reverse-DNS identifier.
    pub const PLUGIN_ID: Word = "a plugin id";
    /// A plugin's readable name.
    pub const PLUGIN_NAME: Word = "a plugin name";
    /// Who a plugin says wrote it.
    pub const PLUGIN_VENDOR: Word = "a plugin vendor";
    /// One id in a required-extension list.
    pub const REQUIRED_EXTENSION: Word = "a required extension id";
    /// One plugin's panel, counted in the headings over its rows.
    pub const PANEL: Word = "a panel";
    /// One plugin's own report, counted in the lines it carries.
    pub const PLUGIN_REPORT: Word = "a plugin report";
    /// One control's identifier.
    pub const CONTROL_ID: Word = "a control id";
    /// What a person reads beside a control.
    pub const CONTROL_LABEL: Word = "a control label";
    /// One dropdown, counted in its options or in the rules between them.
    pub const DROPDOWN: Word = "a dropdown";
    /// One label in a dropdown's menu.
    pub const DROPDOWN_OPTION: Word = "a dropdown option";
    /// One file row, counted in the extensions its dialog offers.
    pub const FILE_CONTROL: Word = "a file control";
    /// One extension in a file row's filter, or what the dialog calls the set.
    pub const FILE_FILTER: Word = "a file filter";
    /// One tone curve, counted in its points.
    pub const TONE_CURVE: Word = "a tone curve";
    /// One heading's identifier.
    pub const HEADING_ID: Word = "a heading id";
    /// What a person reads on a twirl header.
    pub const HEADING_LABEL: Word = "a heading label";
}

/// One of [`INTERNED`]'s words, as a refusal's field type.
///
/// **An alias, and it is doing work.** serde's derive reads a field's type as
/// it is written and adds a lifetime bound for every lifetime it finds there,
/// so a field spelled `&'static str` makes the generated impl demand that a
/// deserialiser's own borrow outlive the program - which nothing can, and which
/// then fails in every struct that holds a refusal rather than here. The word
/// is genuinely `&'static`, because it is one of this crate's own constants;
/// naming it through an alias is what says so to a reader and nothing to the
/// derive.
///
/// *ponytail:* the honest fix is a typed enumeration per vocabulary - a
/// `Subject` and a `Field` - which would make an unreserved word unwritable
/// rather than merely unreadable. It is a wider change than this package, and
/// [`INTERNED`] plus `every_compared_field_is_a_word_a_refusal_can_carry` is
/// what holds the line until then.
pub type Word = &'static str;

/// How a refusal's own [`Word`] crosses the pipe.
///
/// Written out because serde has no `Deserialize` for `&'static str` and cannot
/// have one: the borrow a deserialiser can lend does not outlive the buffer it
/// came from. What can be done is to read the word and find it among the ones
/// this crate wrote, which is what [`INTERNED`] is for. A word that is not
/// there is an error rather than a fallback - a fallback would print a sentence
/// about a field nobody named.
mod interned {
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serializer};

    /// Write the word.
    pub fn serialize<S: Serializer>(value: &super::Word, into: S) -> Result<S::Ok, S::Error> {
        into.serialize_str(value)
    }

    /// Read it back as the one this crate already holds.
    pub fn deserialize<'de, D: Deserializer<'de>>(from: D) -> Result<super::Word, D::Error> {
        let word = String::deserialize(from)?;
        super::INTERNED
            .into_iter()
            .find(|known| *known == word)
            .ok_or_else(|| D::Error::custom(format!("{word:?} is not a word a refusal carries")))
    }
}

/// One of the ceilings the frozen header declares, named by what it counts.
///
/// The header declares its limits rather than leaving each reader to invent
/// one, because a number invented locally cannot be raised once a vendor has
/// shipped inside it, nor narrowed once one has shipped up against it
/// (docs/impl/lfx.md §12). This is the host's side of the same numbers: a name
/// from a closed list, so the Addons page, the scan report and `lfx-validator`
/// all read the same word for the same limit rather than three sentences.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Ceiling {
    /// `LFX_MAX_STRING_BYTES` - how long any one declared string may be: an
    /// identifier, a name, a vendor, an extension id.
    StringBytes,
    /// `LFX_MAX_CATEGORIES` - how many picture families one plugin may claim.
    Categories,
    /// `LFX_MAX_REQUIRED_EXTENSIONS` - how many extensions it may say it cannot
    /// run without.
    RequiredExtensions,
    /// `LFX_MAX_EFFECTS_PER_BUNDLE` - how many plugins one bundle may hold.
    EffectsPerBundle,
    /// `LFX_MAX_OPTIONS` - how many labels one dropdown may offer.
    Options,
    /// `LFX_MAX_DIVIDERS` - how many rules it may draw between them.
    Dividers,
    /// `LFX_MAX_FILTERS` - how many extensions one file row's dialog offers.
    Filters,
    /// `LFX_MAX_CURVE_POINTS` - how many points one tone curve carries.
    CurvePoints,
    /// `LFX_MAX_PARAMS` - how many declarations one effect may push, and so
    /// how many headings may stand over them and how many lines its own report
    /// may carry.
    Params,
}

impl Ceiling {
    /// The header's own number for this ceiling, read from `lumit-lfx-abi`
    /// rather than repeated here.
    #[must_use]
    pub const fn limit(self) -> u64 {
        match self {
            Ceiling::StringBytes => LFX_MAX_STRING_BYTES as u64,
            Ceiling::Categories => LFX_MAX_CATEGORIES as u64,
            Ceiling::RequiredExtensions => LFX_MAX_REQUIRED_EXTENSIONS as u64,
            Ceiling::EffectsPerBundle => LFX_MAX_EFFECTS_PER_BUNDLE as u64,
            Ceiling::Options => LFX_MAX_OPTIONS as u64,
            Ceiling::Dividers => LFX_MAX_DIVIDERS as u64,
            Ceiling::Filters => LFX_MAX_FILTERS as u64,
            Ceiling::CurvePoints => LFX_MAX_CURVE_POINTS as u64,
            Ceiling::Params => LFX_MAX_PARAMS as u64,
        }
    }

    /// What it counts, for the sentence the page prints.
    #[must_use]
    pub const fn counts(self) -> &'static str {
        match self {
            Ceiling::StringBytes => "bytes",
            Ceiling::Categories => "categories",
            Ceiling::RequiredExtensions => "required extensions",
            Ceiling::EffectsPerBundle => "plugins",
            Ceiling::Options => "options",
            Ceiling::Dividers => "rules",
            Ceiling::Filters => "filter extensions",
            Ceiling::CurvePoints => "points",
            Ceiling::Params => "declarations",
        }
    }
}

impl std::fmt::Display for Ceiling {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.counts())
    }
}

/// Why a plugin, or one of its declarations, is refused.
///
/// Named refusals rather than strings, so the validator can test for one by
/// name and the page can print it without parsing anything
/// (docs/14-ENGINEERING-RULES.md §4). The OFX host's `Rejection` is the same
/// shape at the same seam.
///
/// The list grows with the packages: the namespace wiring brought the version
/// arithmetic's own refusal, the describe lowering its own, the protocol the
/// header's declared ceilings the untrusted direction of the pipe is held to,
/// and the manifest check and the extension negotiation each bring theirs
/// (docs/impl/lfx.md §2.3, §3.3, §4.3).
///
/// [`PartialEq`] without [`Eq`]: a declared range end is an `f64` and reaches
/// [`LfxRejection::RangeUnusable`] as the plugin wrote it, so that the sentence
/// on the page names the numbers rather than describing them.
///
/// A refusal is a thing the broker files and the host reads, so the whole
/// enumeration goes on the wire (docs/impl/lfx.md §3.2): the sink runs in the
/// second process and its report lines have to reach the page in the first.
/// Typed across the boundary rather than rendered into sentences there, so that
/// `lfx-validator` can ask for one by name on the side that receives it.
#[derive(Clone, Debug, Deserialize, Error, PartialEq, Serialize)]
pub enum LfxRejection {
    /// A release number the frame key could not tell apart from another one.
    ///
    /// The minted version is `major × 1 000 000 + minor × 1 000 + patch`
    /// ([`crate::version::mint`]), which is one number per release only while
    /// the minor and the patch each stay under a thousand and the major stays
    /// under [`crate::version::MAJOR_LIMIT`] - the first major whose own block
    /// of a million does not fit the `u32` whole, which is one release short
    /// of where the arithmetic gives out.
    /// Refusing all three at describe is what keeps the arithmetic injective,
    /// rather than hoping no vendor ever ships a patch 1 500 and quietly
    /// serves it the frames of (minor + 1, patch 500).
    #[error(
        "the version {major}.{minor}.{patch} is outside the range the frame key can tell \
         releases apart in: the major must be under {}, the minor and the patch under {}",
        crate::version::MAJOR_LIMIT,
        crate::version::COMPONENT_LIMIT
    )]
    VersionOutOfRange {
        /// The major number, as declared.
        major: u32,
        /// The minor number, as declared.
        minor: u32,
        /// The patch number, as declared.
        patch: u32,
    },

    // ------------------------------------------------ structural refusals --
    /// Two rows would land on the same
    /// [`ParamId`](lumit_core::fx::ParamId).
    ///
    /// `ParamId` is a const FNV-1a hash of the row's id, and the resolved value
    /// bag has only the hashes: two rows sharing one is one control silently
    /// driving another. A plugin's parameter names are not ours to choose, so
    /// the collision is made loud at describe rather than shipped as an
    /// ambiguity.
    #[error("two controls share the id {first:?} (the second is {second:?}), which would be one of them driving the other")]
    DuplicateParamId {
        /// The row that got there first.
        first: String,
        /// The row that collided with it.
        second: String,
    },

    /// A declaration on an id the host writes itself.
    ///
    /// [`DERIVED_PREFIX`](crate::def::DERIVED_PREFIX) is the host's own corner
    /// of the resolved bag: `LfxDef::resolve_derived` pushes `derived.frame`
    /// into it on every render, and the built-ins beside it push a dozen more.
    /// A plugin declaring a control by one of those names would draw a panel
    /// row whose value is overwritten before the plugin ever reads it - a row
    /// that silently does nothing, with no line and no refusal to say why. The
    /// prefix is a short, frozen vocabulary this project owns, so the collision
    /// is made loud at describe rather than shipped as a dead control.
    #[error(
        "the control {id:?} is declared inside {:?}, which is the prefix the host's own values are pushed under",
        crate::def::DERIVED_PREFIX
    )]
    ReservedParamId {
        /// The id the plugin declared.
        id: String,
    },

    /// A control declared [`LFX_UNIT_UNSET`](lumit_lfx_abi::LFX_UNIT_UNSET).
    ///
    /// Unit is mandatory (docs/impl/lfx.md D4). A dimensionless control
    /// declares [`LFX_UNIT_RAW`](lumit_lfx_abi::LFX_UNIT_RAW) deliberately, so
    /// "nobody decided" and "no unit" are different answers - which is the
    /// whole of what `every_parameter_declares_a_deliberate_unit` refuses to
    /// let a built-in ship, mirrored here for a stranger's effect.
    #[error("the control {id:?} declared no unit, and a unit cannot be guessed")]
    UnitUnset {
        /// The control that declared none.
        id: String,
    },

    /// A control declared
    /// [`LFX_UNIT_PCT_DIAG`](lumit_lfx_abi::LFX_UNIT_PCT_DIAG).
    ///
    /// The constant exists for the region-of-interest padding declaration and
    /// for the reference format; no parameter may be in it, because every
    /// distance in Lumit is pixels at composition size. The built-ins meet the
    /// same refusal in `no_parameter_is_a_per_cent_of_the_diagonal`.
    #[error("the control {id:?} is a per cent of the diagonal, which no control may be in: every distance here is pixels at composition size")]
    UnitPctDiag {
        /// The control that declared it.
        id: String,
    },

    /// A declared temporal window the host cannot honour.
    ///
    /// The pair is the *gate*: it says which neighbouring frames the effect
    /// reads, so it must contain the frame being rendered
    /// (`lo <= 0 <= hi`) and neither end may reach past
    /// [`LFX_MAX_TEMPORAL_WINDOW`](lumit_lfx_abi::LFX_MAX_TEMPORAL_WINDOW).
    /// A window that does neither is a declaration nobody can carry out rather
    /// than an ambitious one, so the header refuses it instead of clamping it:
    /// a clamp would quietly hand the effect a different window from the one it
    /// says it reads, which is the tile-seam bug docs/13:297 names.
    #[error(
        "the temporal window [{lo}, {hi}] is not one the host can honour: it must contain the frame being \
         rendered and reach at most {} frames either way",
        lumit_lfx_abi::LFX_MAX_TEMPORAL_WINDOW
    )]
    TemporalWindowUnusable {
        /// The first frame read, relative to the one being rendered.
        lo: i32,
        /// The last frame read, relative to the one being rendered.
        hi: i32,
    },

    /// More declarations than one effect may push into the sink.
    ///
    /// [`LFX_MAX_PARAMS`](lumit_lfx_abi::LFX_MAX_PARAMS) is the ABI's own
    /// ceiling, and it refuses the effect rather than costing the last row its
    /// control: the rows it mints are leaked for the session and the panel has
    /// to draw all of them, so a describe loop that ran away is a plugin the
    /// host cannot represent rather than one control short. Recorded once
    /// however long the loop goes on, since a line per declaration past the
    /// ceiling would be the same unbounded growth by another road.
    ///
    /// **Counted over declarations pushed, not rows accepted**, which is the
    /// whole of what the ceiling is for. A loop of declarations the sink
    /// *declines* - a dropdown with nothing in it, a heading inside a heading,
    /// a kind this version reserves - grows the report by an owned string a
    /// call while the panel stays empty, so a gate reading the accepted count
    /// would never trip on the runaway it was added to stop. Controls and
    /// headings therefore share one budget.
    #[error(
        "the effect pushed {declared} declarations, and at most {} may be declared",
        lumit_lfx_abi::LFX_MAX_PARAMS
    )]
    TooManyParams {
        /// How many had been pushed when the ceiling was reached.
        declared: u32,
    },

    /// More of something than the frozen header admits.
    ///
    /// The broker's direction of the pipe is a stranger's: a manifest, a
    /// descriptor and a log line all arrive from code Lumit did not write, and
    /// the only thing bounding them before this is the transport's own 8 MiB
    /// cap - which is four thousand names at a kilobyte each, or of the order
    /// of a hundred thousand plugin records the Addons page would list and the
    /// roster would store. So the untrusted direction is held to the header's
    /// own ceilings as it comes off the pipe
    /// ([`BrokerMessage::checked`](crate::ipc::proto::BrokerMessage::checked)).
    ///
    /// **The sentence never quotes the stranger's own string.** The refusal is
    /// *about* a string nobody has checked yet, so `subject` is one of a closed
    /// set of words this crate wrote rather than the declaration itself.
    ///
    /// The describe lowering meets some of the same numbers from the other side
    /// and answers differently - [`LfxRejection::TooManyCategories`] and
    /// [`LfxRejection::TooManyRequiredExtensions`] are report lines, because a
    /// descriptor whose category list this host will not read is still an
    /// effect, one heading short. This one is the wire's own answer, taken
    /// before a byte is trusted, so it refuses.
    #[error("{subject} holds {given} {ceiling}, past the {} the header admits", .ceiling.limit())]
    PastCeiling {
        /// Which of the header's ceilings.
        ceiling: Ceiling,
        /// What was measured, in this crate's own words.
        #[serde(with = "interned")]
        subject: Word,
        /// What it came to.
        given: u64,
    },

    /// One of the descriptor's **own** strings, past the ABI's ceiling.
    ///
    /// Every `const char *` the ABI carries is bounded by
    /// [`LFX_MAX_STRING_BYTES`](lumit_lfx_abi::LFX_MAX_STRING_BYTES), and a
    /// row's id, label and option text are asked for it in the sink. The
    /// descriptor's three are asked for it at the lowering, because that is
    /// where they arrive: a `PluginDescriptor` need not have come through a
    /// sink at all.
    ///
    /// Structural, where a row's is a line: a row this build cannot draw costs
    /// that one control, while a plugin whose id, name or vendor cannot be
    /// carried has no nameable identity - and the id is what the match name a
    /// project file stores is minted from.
    #[error(
        "the plugin's {field} is {bytes} bytes, and at most {} are carried, so it is not an effect here",
        lumit_lfx_abi::LFX_MAX_STRING_BYTES
    )]
    IdentityStringTooLong {
        /// Which of the descriptor's strings it was: the id, the name or the
        /// vendor.
        field: String,
        /// How long it was, the NUL counted.
        bytes: u32,
    },

    // ---------------------------------------------------- report lines --
    /// A bezier path row, which version 1 reserves.
    ///
    /// The discriminant exists from day one so that admitting it when
    /// `lfx.overlay` lands adds no variant and breaks no compiled plugin. Until
    /// then a path with no on-Viewer handles is a control nobody can edit.
    #[error("the path control {id:?} waits for the Viewer overlay, so it is not drawn and keeps the default it was declared with")]
    PathNeedsOverlay {
        /// The control that declared it.
        id: String,
    },

    /// A text row, which version 1 reserves.
    ///
    /// The resolved value bag carries no text at all, so the value would never
    /// reach `process`: the row would be a control whose setting nothing reads.
    /// *ponytail:* a `ParamKind::Text` would take one bag variant and one panel
    /// widget, and is the whole of what admitting it needs.
    #[error("the text control {id:?} has no row here, so it is not drawn and keeps the default it was declared with")]
    NoTextRow {
        /// The control that declared it.
        id: String,
    },

    /// A kind tag this version of the host does not admit at all.
    ///
    /// The frozen sink has an entry point per kind it *does* admit, so no
    /// plugin can reach this through the ABI. Its caller is whoever decodes a
    /// declaration that never came through that sink - the broker, reading a
    /// proto message whose kind tag is `LFX_PARAM_UNSET`, a `GROUP` where a row
    /// belongs, or a number from a header this build has never seen. Named
    /// rather than folded into the path refusal, so the report never calls a
    /// control something it is not.
    #[error("the control {id:?} was declared as kind {declared}, which this version does not admit, so it is not drawn")]
    UnknownParamKind {
        /// The kind tag, as the declaration carried it.
        declared: lumit_lfx_abi::LfxParamKind,
        /// The control that carried it.
        id: String,
    },

    /// A string past the ABI's own ceiling.
    ///
    /// Every `const char *` the ABI carries is bounded by
    /// [`LFX_MAX_STRING_BYTES`](lumit_lfx_abi::LFX_MAX_STRING_BYTES), the NUL
    /// counted, and the header says the host enforces the same numbers where a
    /// stranger's bytes arrive. A row whose id, label or option text runs past
    /// it is a row this panel cannot draw, so it costs that one control.
    #[error(
        "the control {id:?} declared a string of {bytes} bytes, and at most {} are carried, so it is not drawn",
        lumit_lfx_abi::LFX_MAX_STRING_BYTES
    )]
    StringTooLong {
        /// The control that declared it.
        id: String,
        /// How long the string was, the NUL counted.
        bytes: u32,
    },

    /// Bytes the host could not read as the thing they claim to be: a size
    /// prefix smaller than the struct this header declares, or an identifier
    /// that is absent or has no end inside `LFX_MAX_STRING_BYTES`. A
    /// descriptor whose own id is either is the same answer with
    /// `LFX_PARAM_UNSET` for its kind, an effect nobody can name being one
    /// nothing else can stand in for.
    ///
    /// **The sentence never quotes the stranger's own string**, for the reason
    /// [`LfxRejection::PastCeiling`] gives: the refusal is about bytes nobody
    /// has been able to read, so all it may name is the kind tag and the size
    /// prefix the declaration carried. Where the prefix was not the fault -
    /// a declaration that is null, or whose identifier has no end inside the
    /// ceiling - the number is nought, because a number that claims to be a
    /// size prefix and is not sends the reader after the wrong fault.
    ///
    /// It is the growth mechanism's other edge. §2.1's rule is that a side
    /// reading a struct it was not built against stops at the bytes it
    /// recognises - which works upwards, where the tail is simply not read, and
    /// cannot work downwards: a prefix shorter than this header's names fields
    /// that are not there, and reading them would be reading somebody else's
    /// memory. Version 1 is the first version, so no honest plugin can declare
    /// one. A report line rather than a refusal, because one unreadable
    /// declaration costs that row its control and nothing else.
    #[error(
        "a declaration of kind {kind} carrying {bytes} bytes could not be read, so it is not drawn"
    )]
    UnreadableDeclaration {
        /// The kind tag the sink entry point it arrived through mints.
        kind: lumit_lfx_abi::LfxParamKind,
        /// The size prefix it carried, or nought where the size prefix was not
        /// what failed.
        bytes: u32,
    },

    /// A dropdown offering more options than the ABI carries.
    #[error(
        "the dropdown {id:?} offers {declared} options, and at most {} are carried, so it is not drawn",
        lumit_lfx_abi::LFX_MAX_OPTIONS
    )]
    TooManyOptions {
        /// The control that declared it.
        id: String,
        /// How many it declared.
        declared: u32,
    },

    /// A dropdown drawing more rules between its options than the ABI carries.
    #[error(
        "the dropdown {id:?} draws {declared} rules between its options, and at most {} are carried, so it is not drawn",
        lumit_lfx_abi::LFX_MAX_DIVIDERS
    )]
    TooManyDividers {
        /// The control that declared it.
        id: String,
        /// How many it declared.
        declared: u32,
    },

    /// A file row filtering on more extensions than the ABI carries.
    #[error(
        "the file control {id:?} filters on {declared} extensions, and at most {} are carried, so it is not drawn",
        lumit_lfx_abi::LFX_MAX_FILTERS
    )]
    TooManyFilters {
        /// The control that declared it.
        id: String,
        /// How many it declared.
        declared: u32,
    },

    /// A numeric range the panel cannot draw: an end that is not a number, or
    /// a pair the wrong way round.
    ///
    /// The clamp the resolve applies is `max(lo).min(hi)`, so a reversed pair
    /// pins the control to `hi` for every stored value, every keyframe and
    /// every expression result - the row is dead and nothing says why. A
    /// transposed pair is the commonest copy-paste slip there is, so it is
    /// named here rather than lowered.
    #[error("the range [{lo}, {hi}] of the control {id:?} is not one the panel can draw, so it is not drawn")]
    RangeUnusable {
        /// The control that declared it.
        id: String,
        /// The low end, as declared.
        lo: f64,
        /// The high end, as declared.
        hi: f64,
    },

    /// A whole number outside the range a stored value can hold.
    ///
    /// The ABI declares an `INT` row's default and bounds as `int64_t` and the
    /// resolved bag holds an `i32`, so a legal declaration outside that range
    /// would reach `process` as a number the plugin never declared. The row is
    /// declined rather than wrapped: a control whose value cannot be stored
    /// cannot be drawn either.
    #[error("the whole number {id:?} declares a bound outside the range a stored value holds, so it is not drawn")]
    WholeNumberOutOfRange {
        /// The control that declared it.
        id: String,
    },

    /// A unit declared on a kind whose unit is fixed.
    ///
    /// A switch, a dropdown, a colour, a seed, a tone curve, a file and a
    /// button are in no unit, and an angle is in degrees by definition, so the
    /// lowering normalises what the plugin said. The normalisation is a line in
    /// the report rather than a silence: the header says an angle's unit *must*
    /// be `LFX_UNIT_DEGREES`, and an author whose declaration was ignored is
    /// owed the sentence saying so.
    #[error("the control {id:?} declared the unit {declared:?}, which its kind does not carry, so the kind's own unit is used")]
    UnitIgnoredForKind {
        /// The control that declared it.
        id: String,
        /// The unit it declared.
        declared: Unit,
    },

    /// A declared temporal window wider than the ring has slots to hold.
    ///
    /// **The ceiling-shaped no.** `LFX_MAX_TEMPORAL_WINDOW` is sixty-four each
    /// way and [`RING_MAX_SLOTS`](crate::ipc::ring::RING_MAX_SLOTS) is
    /// sixty-four slots, so a window wider than ±31 is held to the ring's own
    /// ceiling before the ledger is asked anything at all. The plugin still
    /// runs and still declares what it declares; what it does not get is a slot
    /// for every neighbour at once.
    ///
    /// **This is a line about a ceiling, and the ceiling refuses.** Nothing
    /// stages a wide prefetch across more journeys - version 1 has no
    /// frames-request seam to stage it through, and `Broker::process` ships
    /// every neighbour the job carries in one request - so a job asking for
    /// more pictures than the ring has slots comes back
    /// `BrokerError::RingTooSmall`, and that frame renders identity with a
    /// badge. What the line is for is the Addons page: the effect is hosted,
    /// every frame whose shipment fits renders, and an operator reading it can
    /// see why the wide ones do not. `slots_for` returns a number and cannot
    /// say which kind of no it gave; this is where the two are told apart
    /// (docs/impl/lfx.md §3.4).
    #[error(
        "a window of {wanted} frames was declared, and the ring holds {slots} slots, so a frame \
         asking for more neighbours than that is refused"
    )]
    WindowHeldToTheRing {
        /// How many slots the declared window asked for.
        wanted: u32,
        /// How many the ring may hold at most.
        slots: u32,
    },

    /// A ring the governor's ledger would not grant whole.
    ///
    /// **The budget-shaped no**, and the twin of
    /// [`LfxRejection::WindowHeldToTheRing`]. The slot count is halved until the
    /// ledger grants it or it reaches the floor, which is taken whatever the
    /// ledger says - a ring of two cannot hold one input, one output and one in
    /// flight at once. The effect is hosted and every frame whose shipment fits
    /// the narrowed ring renders; a shipment wider than it has slots for is
    /// refused whole, as `BrokerError::RingTooSmall`, for as long as the
    /// pressure lasts. The line says which of the two ceilings did it, so that
    /// a page reading it is told what the refusal is telling the layer.
    #[error(
        "a ring of {wanted} slots was asked for and the ledger granted {granted}, so a frame \
         asking for more neighbours than that is refused"
    )]
    RingNarrowedByTheLedger {
        /// How many slots were asked for, after every ceiling had its say.
        wanted: u32,
        /// How many the ledger paid for.
        granted: u32,
    },

    /// A row declared
    /// [`LFX_PARAM_FLAG_STATIC`](lumit_lfx_abi::LFX_PARAM_FLAG_STATIC) on a
    /// kind this build lets a person keyframe.
    ///
    /// The flag says the row never keyframes - "one value for the whole of the
    /// effect's life, as a file choice or a curve is". A tone curve, a file
    /// choice and a button are already that, so the flag is redundant on them
    /// and says nothing. On every other kind it is a declaration this build has
    /// nowhere to put: [`ParamSchema`](lumit_core::fx::ParamSchema) carries no
    /// non-animatable field, so the row is drawn keyframeable and a person may
    /// animate it against the plugin's own words. The line is here rather than
    /// a silence for the reason
    /// [`LfxRejection::UnitIgnoredForKind`] exists: a declared field whose
    /// value is thrown away without a word is the fault left to be found
    /// somewhere later.
    ///
    /// *ponytail:* a `ParamSchema` flag, read by the panel and by the keyframe
    /// menu, is the whole of what honouring the declaration needs.
    #[error("the control {id:?} was declared static, and this build has no way to stop it keyframing, so the flag is ignored")]
    StaticRowAnimatesAnyway {
        /// The control that declared it.
        id: String,
    },

    /// A dropdown whose default selects an option it has not got.
    ///
    /// The default crosses as an index into the option list and the panel draws
    /// whatever sits there, so an index past the end is a dropdown that opens
    /// with nothing selected and a value the plugin never offered. Lumit
    /// declares this list rather than inheriting somebody else's, so the
    /// declaration is ours to hold to its own numbers.
    #[error("the dropdown {id:?} starts on option {declared}, and it offers {options}, so it is not drawn")]
    ChoiceDefaultOutOfRange {
        /// The control that declared it.
        id: String,
        /// The option index it starts on.
        declared: u32,
        /// How many options it offers.
        options: u32,
    },

    /// A dropdown drawing a rule after an option it has not got.
    ///
    /// A divider is declared as the index it follows, so an index at or past
    /// the end names no option. Declined whole rather than trimmed, which is
    /// the answer [`LfxRejection::TooManyDividers`] already gives for the same
    /// list: the header's own words are that a ceiling here "declines the whole
    /// declaration rather than trimming it".
    #[error("the dropdown {id:?} draws a rule after option {declared}, and it offers {options}, so it is not drawn")]
    DividerPastTheOptions {
        /// The control that declared it.
        id: String,
        /// The divider index that names no option.
        declared: u32,
        /// How many options it offers.
        options: u32,
    },

    /// More picture families than the closed vocabulary has.
    ///
    /// Where the count arrives beside the array it counts - a descriptor read
    /// through the ABI - the list is **not read at all**, the count having
    /// failed before anything behind it could be trusted. Where the list is
    /// already in hand, off the wire, the first
    /// [`LFX_MAX_CATEGORIES`](lumit_lfx_abi::LFX_MAX_CATEGORIES) are kept.
    /// Either way this line is the sentence that says the declaration was not
    /// honoured as written.
    #[error(
        "{declared} picture families were declared, and at most {} exist, so the list is not read as declared",
        lumit_lfx_abi::LFX_MAX_CATEGORIES
    )]
    TooManyCategories {
        /// How many the descriptor declared.
        declared: u32,
    },

    /// More required extensions than the ABI carries.
    ///
    /// The list decides whether the plugin is instantiated at all
    /// (docs/impl/lfx.md §4.3), so the overflow is named here rather than
    /// dropped quietly - and, as with
    /// [`LfxRejection::TooManyCategories`], a count that arrives beside its own
    /// array stops the array being read at all rather than being clamped to
    /// the ceiling and read to it.
    #[error(
        "{declared} required extensions were declared, and at most {} are carried, so the list is not read as declared",
        lumit_lfx_abi::LFX_MAX_REQUIRED_EXTENSIONS
    )]
    TooManyRequiredExtensions {
        /// How many the descriptor declared.
        declared: u32,
    },

    /// Two descriptors in one bundle declaring the same id.
    ///
    /// [`LfxRejection::DuplicateParamId`] one level up, and the same argument:
    /// the id is what a saved project, a match name and a frame key resolve
    /// through, so two effects sharing one is a bundle where which effect a
    /// project opens is decided by the descriptor list's order. The first is
    /// catalogued and the later one is dropped, which is a line about the
    /// bundle rather than the end of it: an author's own mistake in one
    /// descriptor should not cost the other eleven their rows.
    #[error("two effects in this bundle declare the id {id:?}, so the second is not catalogued")]
    DuplicateEffectId {
        /// The id declared twice.
        id: String,
    },

    /// The bundle's manifest and the bundle's own code say different things
    /// about one plugin.
    ///
    /// **The manifest is the cheap listing, never the authority**
    /// (docs/impl/lfx.md §3.3, §11 item 6). It is read before any of the
    /// plugin's code runs, which is what lets the Addons page name a plugin
    /// that has never started; the descriptor is what the code itself answers,
    /// and where the two disagree the code wins and the plugin is refused.
    ///
    /// Refused rather than reported, because every field this compares is one
    /// something downstream resolves through: the id is the match name, the
    /// three version numbers are the frame key, and the required-extension list
    /// decides whether the plugin is instantiated at all. A bundle declaring
    /// `required = []` and in fact asking for `lfx.temporal` would pass
    /// negotiation, reach `create`, get a null and fail somewhere later - which
    /// is the outcome the negotiation exists to prevent (§4.3).
    #[error(
        "the bundle's manifest says this plugin's {field} is {manifest:?} and its own code says \
         {code:?}, and the code is the authority"
    )]
    ManifestMismatch {
        /// Which plugin, by the id the manifest gave it.
        id: String,
        /// Which field disagreed, from the closed list
        /// [`crate::manifest::COMPARED_FIELDS`].
        #[serde(with = "interned")]
        field: Word,
        /// What the manifest declared.
        manifest: String,
        /// What the descriptor declared.
        code: String,
    },

    /// The plugin's own `describe` answered `false`.
    ///
    /// A refusal rather than an absence, and that is the whole of what it buys.
    /// The describe runs in the broker, so a plugin that declines to say what
    /// it is would otherwise reach the host as a plugin that is simply not in
    /// the list - indistinguishable from one the user switched off, with no
    /// sentence for §5.3's `REFUSED` table to carry. The bundle carries on:
    /// eleven good effects are not a bundle to throw away because the twelfth
    /// declined.
    #[error("the effect {id:?} refused to describe itself")]
    DescribeRefused {
        /// The effect that refused.
        id: String,
    },

    /// The effect cannot work without an extension this host does not offer.
    ///
    /// Refused **before** it is instantiated rather than left to fail somewhere
    /// later, with the extension named, which is §4.3's whole argument: a
    /// plugin that reached `create`, got a `NULL` from `get_extension` and
    /// failed at process time would be the "left to fail somewhere later"
    /// outcome the negotiation exists to prevent.
    #[error(
        "the effect {id:?} requires the extension {extension:?}, which this host does not offer"
    )]
    RequiresExtension {
        /// The effect that asked.
        id: String,
        /// What it asked for.
        extension: String,
    },

    /// A plugin the module holds that could not be asked what it is, for a
    /// reason with no name of its own.
    ///
    /// The describe runs in the second process and the faults it can meet there
    /// are [`crate::local::LocalError`]'s - a module that would not load, an
    /// instance table shorter than this header, a `create` that answered null.
    /// Most of those already have a refusal of their own and cross as it; this
    /// is the arm for the rest, and it carries the host's own sentence rather
    /// than a stranger's string, cut to `LFX_MAX_LOG_BYTES` where it is minted.
    ///
    /// *ponytail:* the fuller answer is for `LocalError` itself to cross, which
    /// means making every arm of it serialisable - a wider change than the one
    /// sentence the Addons page needs.
    #[error("the effect {id:?} could not be described: {why}")]
    DescribeFailed {
        /// The effect that could not be described.
        id: String,
        /// The host's own sentence for why.
        why: String,
    },

    /// A category outside the closed picture-family vocabulary.
    #[error("the category {declared} is not one of the eight picture families, so the effect is filed under Utility")]
    UnknownCategory {
        /// The number the plugin declared.
        declared: u32,
    },

    /// A descriptor that named no picture family at all.
    ///
    /// An empty list, and a list of nothing but
    /// [`LFX_CATEGORY_UNSET`](lumit_lfx_abi::LFX_CATEGORY_UNSET), are the same
    /// statement: the vendor filled nothing in. The unset value is therefore
    /// not named as an [`LfxRejection::UnknownCategory`], which would read as a
    /// number from a header this build has never seen.
    #[error("no picture family was declared, so the effect is filed under Utility")]
    NoCategoryDeclared,

    /// A padded region of interest with nothing to pad by.
    #[error("a padded region of interest was declared with no distance, so the effect is read as needing one input pixel per output pixel")]
    PaddingWithoutDistance,

    /// A dropdown with nothing in it.
    #[error("the dropdown {id:?} offers nothing to choose, so it is not drawn")]
    ChoiceWithNoOptions {
        /// The control that declared it.
        id: String,
    },

    /// A tone curve with too few or too many control points.
    #[error(
        "the tone curve {id:?} declared {declared} points, and a curve carries between {} and {}, so it is not drawn",
        lumit_lfx_abi::LFX_MIN_CURVE_POINTS,
        lumit_lfx_abi::LFX_MAX_CURVE_POINTS
    )]
    CurvePointsOutOfRange {
        /// The control that declared it.
        id: String,
        /// How many points it declared.
        declared: u32,
    },

    /// A heading opened inside another.
    ///
    /// A [`ParamGroup`](lumit_core::fx::ParamGroup) is one contiguous run of
    /// rows drawn under one heading, and a nested run would split the outer one
    /// in two - which draws the outer heading twice. So the inner heading is
    /// declined and its rows join the one already open.
    #[error("the heading {id:?} opened inside another, and headings here do not nest, so its rows join the heading already open")]
    GroupInsideGroup {
        /// The heading that was declined.
        id: String,
    },

    /// A heading closed with none open.
    #[error("a heading was closed with none open")]
    GroupEndWithNothingOpen,

    /// A heading the plugin never closed.
    #[error("the heading {id:?} was never closed, so it ends where the declarations do")]
    GroupLeftOpen {
        /// The heading left open.
        id: String,
    },
}

impl LfxRejection {
    /// Whether this refusal ends the effect, or is a line in the scan report
    /// beside an effect that still loads.
    ///
    /// docs/12 §3.6 asks for both, and they are different answers to different
    /// faults: a control this build cannot draw costs that one row its control
    /// and nothing else, while two rows driving each other or a unit nobody
    /// stated is a fault no amount of graceful degradation makes safe.
    ///
    /// Written as an exhaustive `match` with no `_` arm, so a variant added by
    /// a later package has to decide which kind it is rather than inheriting an
    /// answer - and, having decided, has one more place to go: the sweep in
    /// `a_refusal_is_either_a_report_line_or_the_end_of_the_effect`, which is
    /// two written-out lists of one witness each and cannot be derived from
    /// this match.
    #[must_use]
    pub const fn refuses_the_effect(&self) -> bool {
        match self {
            LfxRejection::VersionOutOfRange { .. }
            | LfxRejection::DuplicateParamId { .. }
            | LfxRejection::ReservedParamId { .. }
            | LfxRejection::UnitUnset { .. }
            | LfxRejection::UnitPctDiag { .. }
            | LfxRejection::TemporalWindowUnusable { .. }
            | LfxRejection::TooManyParams { .. }
            | LfxRejection::ManifestMismatch { .. }
            | LfxRejection::DescribeRefused { .. }
            | LfxRejection::RequiresExtension { .. }
            | LfxRejection::DescribeFailed { .. }
            | LfxRejection::PastCeiling { .. }
            | LfxRejection::IdentityStringTooLong { .. } => true,
            LfxRejection::PathNeedsOverlay { .. }
            | LfxRejection::NoTextRow { .. }
            | LfxRejection::UnknownParamKind { .. }
            | LfxRejection::StringTooLong { .. }
            | LfxRejection::UnreadableDeclaration { .. }
            | LfxRejection::TooManyOptions { .. }
            | LfxRejection::TooManyDividers { .. }
            | LfxRejection::TooManyFilters { .. }
            | LfxRejection::RangeUnusable { .. }
            | LfxRejection::WholeNumberOutOfRange { .. }
            | LfxRejection::UnitIgnoredForKind { .. }
            | LfxRejection::WindowHeldToTheRing { .. }
            | LfxRejection::RingNarrowedByTheLedger { .. }
            | LfxRejection::StaticRowAnimatesAnyway { .. }
            | LfxRejection::ChoiceDefaultOutOfRange { .. }
            | LfxRejection::DividerPastTheOptions { .. }
            | LfxRejection::TooManyCategories { .. }
            | LfxRejection::TooManyRequiredExtensions { .. }
            | LfxRejection::DuplicateEffectId { .. }
            | LfxRejection::UnknownCategory { .. }
            | LfxRejection::NoCategoryDeclared
            | LfxRejection::PaddingWithoutDistance
            | LfxRejection::ChoiceWithNoOptions { .. }
            | LfxRejection::CurvePointsOutOfRange { .. }
            | LfxRejection::GroupInsideGroup { .. }
            | LfxRejection::GroupEndWithNothingOpen
            | LfxRejection::GroupLeftOpen { .. } => false,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A refusal crosses the pipe and comes back as itself, the two words this
    /// crate wrote included. `bincode` is the format `lumit_ipc::pipe` uses, so
    /// the round trip asked here is the one the broker really makes.
    #[test]
    fn a_refusal_crosses_the_pipe_and_comes_back_as_itself() {
        for refusal in [
            LfxRejection::PastCeiling {
                ceiling: Ceiling::EffectsPerBundle,
                subject: "the module",
                given: 2_000,
            },
            LfxRejection::ManifestMismatch {
                id: "com.example.blur".into(),
                field: "required extensions",
                manifest: String::new(),
                code: "lfx.temporal".into(),
            },
            LfxRejection::UnitIgnoredForKind {
                id: "a".into(),
                declared: Unit::Seconds,
            },
            LfxRejection::RangeUnusable {
                id: "a".into(),
                lo: 1.0,
                hi: 0.0,
            },
        ] {
            let bytes = bincode::serialize(&refusal).expect("a refusal goes on the wire");
            let back: LfxRejection = bincode::deserialize(&bytes).expect("and comes back");
            assert_eq!(back, refusal);
        }
    }

    /// A word off the wire that this crate never wrote ends the conversation
    /// rather than being printed. The alternative - a fallback word - would put
    /// a sentence on the Addons page about a field nobody named.
    #[test]
    fn a_word_no_refusal_carries_is_refused_rather_than_printed() {
        let good = LfxRejection::PastCeiling {
            ceiling: Ceiling::Categories,
            subject: "a plugin",
            given: 9,
        };
        let mut bytes = bincode::serialize(&good).expect("a refusal goes on the wire");
        // The subject is the only string in this message, so replacing it byte
        // for byte with another of the same length leaves a well-formed message
        // carrying a word nobody reserved.
        let at = bytes
            .windows(8)
            .position(|window| window == b"a plugin")
            .expect("the subject is in the bytes");
        bytes.splice(at..at + 8, *b"anything");
        let back: Result<LfxRejection, _> = bincode::deserialize(&bytes);
        assert!(back.is_err(), "an unreserved word was believed");
    }

    /// Every field [`crate::manifest::agrees`] may name is a word a refusal can
    /// carry back. Two lists that must agree and are written twice is how the
    /// next field added to the comparison comes to be unreadable at the other
    /// end of the pipe.
    #[test]
    fn every_compared_field_is_a_word_a_refusal_can_carry() {
        for field in crate::manifest::COMPARED_FIELDS {
            assert!(
                INTERNED.contains(&field),
                "{field:?} is compared but cannot cross the pipe"
            );
        }
    }

    /// Every subject a [`LfxRejection::PastCeiling`] may name is a word a
    /// refusal can carry back, exactly as every compared field is.
    ///
    /// Half of [`INTERNED`] had no list and no sweep until this test: the
    /// subjects were literals at their call sites, and a ninth one added by a
    /// later package would serialise in the broker and fail to deserialise in
    /// the host. `pipe::recv` would then error, the read loop would say the
    /// broker is gone, the strike would restart it, and the deterministic
    /// re-describe would do it twice more - a bundle disabled for the session
    /// over a report line nobody could read.
    #[test]
    fn every_ceiling_subject_is_a_word_a_refusal_can_carry() {
        for subject in SUBJECTS {
            assert!(
                INTERNED.contains(&subject),
                "{subject:?} names a ceiling but cannot cross the pipe"
            );
        }
    }

    /// And no word is reserved twice, which would make the list's length a lie
    /// about how many words there are.
    #[test]
    fn no_word_is_reserved_twice() {
        let unique: std::collections::BTreeSet<&str> = INTERNED.into_iter().collect();
        assert_eq!(unique.len(), INTERNED.len());
    }
}
