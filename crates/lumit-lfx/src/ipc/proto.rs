//! The control protocol: every sentence the host and the broker can say
//! (docs/impl/lfx.md §3.2).
//!
//! # In plain terms
//!
//! Two programs that have to agree need a fixed, small vocabulary and a version
//! number on the front of it. This module is that vocabulary. The host does the
//! asking ([`HostMessage`]) and the broker does the answering
//! ([`BrokerMessage`]), except for two answers that are really questions - the
//! broker asking for frames it has discovered it needs, and the plugin saying
//! something through the host's log.
//!
//! **No pixels travel here.** Every message that involves a picture names a
//! *slot* in the shared-memory ring ([`crate::ipc::ring`]); the bytes are
//! already there. A control message is tens of bytes and a frame is tens of
//! megabytes, and keeping them apart is the whole reason the frame plane
//! exists.
//!
//! **This version is not the ABI's.** [`PROTOCOL_VERSION`] says how the host
//! and its own broker talk to each other, and `LFX_ABI_VERSION` says what a
//! stranger's compiled code was built against. They are two constants in two
//! crates because they change for different reasons: a message gaining a field
//! is a Lumit release talking to itself, and no plugin anywhere has to be
//! rebuilt for it (docs/12 §3.5).
//!
//! **Every host message is answered exactly once**, which is what lets a
//! deadline with no reply be a strike rather than a wait. Two are exempt and
//! both are exempt for a reason [`HostMessage::expects_reply`] spells out:
//! [`HostMessage::Frames`] is consumed inside the exchange loop that asked for
//! it, and [`HostMessage::Shutdown`] is sent when there is nobody left to
//! answer. [`BrokerMessage::Done`] is what the three messages with nothing to
//! report are answered with, and it is not optional: without it each of them
//! would be a guaranteed control-deadline timeout, and a user dragging one
//! slider three times would disable the plugin.
//!
//! **And answered with the answer it admits**, which is
//! [`HostMessage::answers`] and the other half of the same rule. "Exactly once"
//! is a count, and a count cannot tell a reply from the reply to the question
//! before last; a broker one message behind on the pipe answers everything
//! promptly and every answer is wrong. So the question names what may answer
//! it, and the supervisor holds the two together.

use std::collections::BTreeSet;

use lumit_lfx_abi::{
    LfxLogLevel, LfxParamKind, LfxPixelFormat, LFX_LOG_DEBUG, LFX_LOG_ERROR, LFX_LOG_INFO,
    LFX_LOG_TRACE, LFX_LOG_WARN, LFX_PARAM_ANGLE, LFX_PARAM_BOOL, LFX_PARAM_CHOICE,
    LFX_PARAM_COLOUR, LFX_PARAM_CURVE, LFX_PARAM_FILE, LFX_PARAM_FLOAT, LFX_PARAM_INT,
    LFX_PARAM_POINT2, LFX_PARAM_POINT3, LFX_PARAM_SEED, LFX_PARAM_SLIDER, LFX_RGBA_F16,
    LFX_RGBA_F32,
};
use serde::{Deserialize, Serialize};

use crate::describe::{Declaration, Declared, GroupRun, PluginDescriptor};
use crate::rejection::subject;
use crate::{Ceiling, LfxRejection};

/// The version both sides must agree on. Bump it whenever a message changes
/// shape: an old broker beside a new host is a mismatch, not a crash.
///
/// One, and versioned independently of the ABI the plugin was built against -
/// the promise docs/12 §3.5 makes, kept structurally by being two constants in
/// two crates rather than by being remembered.
pub const PROTOCOL_VERSION: u32 = 1;

/// Which instance a message is about. The host mints these; the broker only
/// ever quotes one back (docs/impl/lfx.md §3.5).
pub type InstanceId = u32;

/// Which slot of the ring a picture is in.
pub type Slot = u32;

/// Four samples to a pixel, everywhere: scene-linear RGBA, premultiplied.
pub const CHANNELS: u32 = 4;

/// A rectangle in pixels, `x0`/`y0` included and `x1`/`y1` excluded - the
/// convention `lfx_process`'s own region fields use, spelled once here so that
/// the wire and the slot header cannot disagree with the ABI about which edge
/// is in.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct RectI {
    /// The left edge, included.
    pub x0: i32,
    /// The top edge, included.
    pub y0: i32,
    /// The right edge, excluded.
    pub x1: i32,
    /// The bottom edge, excluded.
    pub y1: i32,
}

impl RectI {
    /// A rectangle of this size at the origin.
    #[must_use]
    pub fn of(width: u32, height: u32) -> Self {
        Self {
            x0: 0,
            y0: 0,
            x1: i32::try_from(width).unwrap_or(i32::MAX),
            y1: i32::try_from(height).unwrap_or(i32::MAX),
        }
    }

    /// How many pixels across. Nought for a rectangle that runs backwards,
    /// which is empty rather than negative.
    #[must_use]
    pub fn width(self) -> u32 {
        u32::try_from(self.x1.saturating_sub(self.x0)).unwrap_or(0)
    }

    /// How many pixels down.
    #[must_use]
    pub fn height(self) -> u32 {
        u32::try_from(self.y1.saturating_sub(self.y0)).unwrap_or(0)
    }

    /// How many samples a tightly packed RGBA picture of this size holds.
    #[must_use]
    pub fn samples(self) -> u64 {
        u64::from(self.width())
            .saturating_mul(u64::from(self.height()))
            .saturating_mul(u64::from(CHANNELS))
    }
}

/// Which depth a picture crosses at.
///
/// Both are mandatory and the host never converts between them to accommodate
/// a plugin (docs/12 §3.3). An 8 bpc project sends fp16: a plugin never sees an
/// integer buffer, and fp16's significand round-trips every 8-bit code value,
/// so the promotion loses nothing the project had (docs/impl/lfx.md §2.5).
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum PixelDepth {
    /// `LFX_RGBA_F16`: the working depth, and half a slot of the other.
    F16,
    /// `LFX_RGBA_F32`: what an fp32 project sends, and what the CPU path reads
    /// back.
    F32,
}

impl PixelDepth {
    /// How many bytes one sample takes - the number the ring sizes a slot with.
    #[must_use]
    pub const fn bytes_per_sample(self) -> u64 {
        match self {
            PixelDepth::F16 => 2,
            PixelDepth::F32 => 4,
        }
    }

    /// The header's own `lfx_pixel_format` for this depth.
    ///
    /// The wire and the ABI must name the same thing by the same number, and
    /// this is where they meet: the broker fills `lfx_frame.format` from it.
    #[must_use]
    pub const fn as_pixel_format(self) -> LfxPixelFormat {
        match self {
            PixelDepth::F16 => LFX_RGBA_F16,
            PixelDepth::F32 => LFX_RGBA_F32,
        }
    }

    /// The depth an `lfx_pixel_format` names, or `None` for `LFX_PIXEL_UNSET`
    /// and for anything a newer header has that this build has not.
    #[must_use]
    pub fn from_pixel_format(format: LfxPixelFormat) -> Option<Self> {
        match format {
            LFX_RGBA_F16 => Some(PixelDepth::F16),
            LFX_RGBA_F32 => Some(PixelDepth::F32),
            _ => None,
        }
    }

    /// The word a report line uses.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            PixelDepth::F16 => "fp16",
            PixelDepth::F32 => "fp32",
        }
    }
}

/// One picture, as the ring slot it is sitting in.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct FrameRef {
    /// The comp frame it is the picture for. The effect's own input carries the
    /// time being rendered; a neighbour carries its own.
    pub time: f64,
    /// Where it is.
    pub slot: Slot,
}

/// One picture the broker has discovered it needs and has not got.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct FrameWanted {
    /// The comp frame wanted.
    pub time: f64,
}

/// How the ring is laid out. Sent after the handshake, and again whenever a
/// frame bigger than a slot arrives ([`crate::ipc::ring`]).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RingSpec {
    /// The backing file both processes map.
    pub path: String,
    /// How many slots it holds. Never fewer than
    /// [`RING_MIN_SLOTS`](crate::ipc::ring::RING_MIN_SLOTS): one being written,
    /// one being read, one between them.
    pub slots: u32,
    /// How many bytes each slot is, header included.
    pub slot_bytes: u64,
    /// The depth the slots were sized for. A frame of the other depth still
    /// fits an fp32-sized ring, and the slot's own header says which one was
    /// written.
    pub depth: PixelDepth,
}

/// One resolved control value, as the dense array element it becomes.
///
/// The arms are `lfx_value`'s union, one for one, and the tag the plugin reads
/// is [`ParamValue::kind`]. Nothing stringly-typed crosses: a kind mismatch is
/// impossible rather than a runtime status (docs/impl/lfx.md §2.2).
///
/// **One declaration is one element**, points included: a `POINT2` crosses as
/// one [`ParamValue::Point2`] carrying both axes, and the host's `_x`/`_y` rows
/// are folded back before the array is written. A declaration the host carries
/// no value for - an Action, a Group - is not in the array at all.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum ParamValue {
    /// `FLOAT`, unbounded or with hard bounds.
    Float(f64),
    /// `SLIDER`, which is a Float with a drawn range and possibly a log scale.
    Slider(f64),
    /// `ANGLE`, in degrees.
    Angle(f64),
    /// `INT`.
    Int(i64),
    /// `SEED`, which is an Int the host holds still between exports.
    Seed(i64),
    /// `BOOL`.
    Bool(bool),
    /// `CHOICE`: the chosen index.
    Choice(u32),
    /// `COLOUR`, scene-linear RGBA.
    Colour([f32; 4]),
    /// `POINT2`: both axes, in one element.
    Point2([f32; 2]),
    /// `POINT3`: all three.
    Point3([f32; 3]),
    /// `CURVE`: the host tone curve, two to sixteen points in the unit square.
    Curve(Vec<[f32; 2]>),
    /// `FILE`: what the auxiliary slot beside the op actually loaded, and
    /// `None` until the generic file aux lands - which is every File row today,
    /// and why the arm is an `Option` rather than a `String`.
    File(Option<String>),
}

impl ParamValue {
    /// The `lfx_param_kind` this value is tagged with.
    ///
    /// Written as an exhaustive match with no `_` arm, so a value added to the
    /// wire without a tag of its own fails this build rather than crossing
    /// untagged.
    #[must_use]
    pub const fn kind(&self) -> LfxParamKind {
        match self {
            ParamValue::Float(_) => LFX_PARAM_FLOAT,
            ParamValue::Slider(_) => LFX_PARAM_SLIDER,
            ParamValue::Angle(_) => LFX_PARAM_ANGLE,
            ParamValue::Int(_) => LFX_PARAM_INT,
            ParamValue::Seed(_) => LFX_PARAM_SEED,
            ParamValue::Bool(_) => LFX_PARAM_BOOL,
            ParamValue::Choice(_) => LFX_PARAM_CHOICE,
            ParamValue::Colour(_) => LFX_PARAM_COLOUR,
            ParamValue::Point2(_) => LFX_PARAM_POINT2,
            ParamValue::Point3(_) => LFX_PARAM_POINT3,
            ParamValue::Curve(_) => LFX_PARAM_CURVE,
            ParamValue::File(_) => LFX_PARAM_FILE,
        }
    }
}

/// What a plugin says it is, in the manifest and again in its descriptor.
///
/// The same record twice on purpose. The manifest is read in the broker before
/// any of the plugin's code runs, which is what lets the Addons page name a
/// plugin it has never started; the descriptor is what the code itself answers.
/// **The manifest is the cheap listing, never the authority** - a disagreement
/// once the module is open is a refusal, and a required-extension list longer
/// than the manifest declared is the one that matters most
/// (docs/impl/lfx.md §3.3, §4.3).
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PluginIdentity {
    /// Reverse-DNS, and stable for the plugin's life. The match name is `lfx:`
    /// and this.
    pub id: String,
    /// What a person reads.
    pub name: String,
    /// Who wrote it.
    pub vendor: String,
    /// The release's major number. [`crate::version::mint`] turns the three
    /// into the one the frame key carries.
    pub major: u32,
    /// The minor number.
    pub minor: u32,
    /// The patch number.
    pub patch: u32,
    /// The picture families it claims, as `lfx_category` values. The first is
    /// the heading; the rest are search keywords (docs/impl/lfx.md §2.4).
    pub categories: Vec<u32>,
    /// Which `LFX_ABI_VERSION` it was built against.
    pub abi_version: u32,
    /// The extensions it cannot run without. Negotiated from the manifest,
    /// before `create`, and re-checked against the descriptor at the first
    /// describe.
    pub required_extensions: Vec<String>,
}

impl PluginIdentity {
    /// The same identity, if every string and every list it carries is inside
    /// the ceilings `lumit-lfx-abi` declares.
    ///
    /// Both halves of the record come off the pipe from a stranger - the
    /// manifest is a stranger's TOML and the descriptor is a stranger's
    /// compiled code - and the Addons page lists what is in them. So the
    /// header's own numbers are read here rather than left to each caller
    /// (docs/impl/lfx.md §12); [`BrokerMessage::checked`] is the gate that
    /// calls it for both answers at once.
    ///
    /// # Errors
    ///
    /// [`LfxRejection::PastCeiling`], naming which ceiling and what it came to.
    pub fn checked(self) -> Result<Self, LfxRejection> {
        bounded_string(subject::PLUGIN_ID, &self.id)?;
        bounded_string(subject::PLUGIN_NAME, &self.name)?;
        bounded_string(subject::PLUGIN_VENDOR, &self.vendor)?;
        bounded_count(Ceiling::Categories, subject::PLUGIN, self.categories.len())?;
        bounded_count(
            Ceiling::RequiredExtensions,
            subject::PLUGIN,
            self.required_extensions.len(),
        )?;
        for extension in &self.required_extensions {
            bounded_string(subject::REQUIRED_EXTENSION, extension)?;
        }
        Ok(self)
    }
}

/// A trait block as the plugin declared it, still in the ABI's own numbers.
///
/// **Raw on purpose.** Lowering `cost`, `roi_kind` and `alpha` onto
/// `CostClass`, `Roi` and the premultiplied flag - and lowering every unstated
/// nought to the *pessimistic* answer rather than to discriminant zero - is
/// [`crate::schema::traits_of`]'s job (docs/impl/lfx.md §2.4). What crosses the
/// pipe is what the plugin said, so the lowering has one place to happen and
/// the broker has no opinion to have.
///
/// This is the wire's copy of [`crate::describe::Traits`], which is the same
/// eight fields in the ABI's own typed aliases. Two structs rather than one
/// because they answer to different things - this one carries serde derives and
/// comes off a stranger's pipe, that one is `const`-constructible straight off
/// `lfx_traits` - and the conversion between them belongs where a `Described`
/// first meets the lowering, which is [`From<DeclaredTraits>`] and its twin
/// below. Nothing may grow a third.
///
/// [`DeclaredTraits::default`] is the zeroed block, which is what a `NULL`
/// trait pointer and a short one both read as, and it is a declaration in its
/// own right: every field's nought means "unstated".
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct DeclaredTraits {
    /// `lfx_cost`; nought is unstated.
    pub cost: u32,
    /// `lfx_roi_kind`; nought is unstated.
    pub roi_kind: u32,
    /// The dilation a padded ROI asks for, in px@comp.
    pub roi_padding_px: f32,
    /// The first frame the effect reads, relative to the one being rendered.
    pub temporal_lo: i32,
    /// The last frame it reads. The pair is the gate, and the ring sizes itself
    /// from it ([`crate::ipc::ring`]).
    pub temporal_hi: i32,
    /// `lfx_alpha`; nought is unstated.
    pub alpha: u32,
    /// `lfx_trait_flags`: seeded, thread-unsafe, cancellable.
    pub flags: u32,
    /// What one megapixel of output costs in working memory - LFX's own
    /// replacement for the host allocator it has not got (docs/impl/lfx.md §8).
    pub scratch_bytes_per_megapixel: u32,
}

impl DeclaredTraits {
    /// The declared window as a pair, held to what can be honoured: it contains
    /// the frame being rendered, and reaches no further either way than the
    /// header's own `LFX_MAX_TEMPORAL_WINDOW`.
    ///
    /// A window that does not contain nought is not an ambitious declaration
    /// but one nobody can honour; the refusal by name belongs at describe,
    /// where the plugin can be told about it. What must not happen is a ring
    /// sized from a number nobody checked.
    ///
    /// *ponytail:* holding is not refusing, and §4.6 is explicit that claiming
    /// less reach than the kernel uses produces tile seams. Here there is
    /// nowhere to say so - this returns a pair, and the ring has to be sized
    /// from something - so the sentence is the describe lowering's:
    /// [`LfxRejection::TemporalWindowUnusable`], which
    /// [`crate::schema::traits_of`] raises for the same window this narrows.
    /// Nothing in this module reaches that refusal, because nothing here has
    /// been through a describe yet. The one that puts the refusal in front of
    /// the narrowing is [`crate::ipc::broker`]'s `admit`, which every declared
    /// window goes through before anything is sized from it.
    #[must_use]
    pub fn temporal_window(self) -> (i32, i32) {
        let limit = lumit_lfx_abi::LFX_MAX_TEMPORAL_WINDOW;
        (
            self.temporal_lo.clamp(-limit, 0),
            self.temporal_hi.clamp(0, limit),
        )
    }
}

/// One plugin, as the broker read it out of the module at the first describe.
///
/// Three things, and each is answered by somebody different. **The identity**
/// is what the code says it is, which the manifest also declared - the two
/// halves §4.3's re-check compares. **The trait block** is what it declared
/// about how it wants scheduling, raw in the ABI's own numbers, which is what
/// §3.4's ring sizes its slots from. And **the declarations** are what the
/// describe sink was pushed, which is the schema lowering's whole vocabulary:
/// the rows in declaration order, the headings over them, and the lines the
/// sink declined.
///
/// The declarations travel because the sink runs in the *other process* - the
/// module is opened there and nowhere else - so a `Described` that carried only
/// the identity would leave the host with a plugin it could name and no rows to
/// put on a panel. `PluginDescriptor::from` is the one conversion back, so the
/// lowering reads one shape whether the plugin was opened here or over a pipe.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct DescribedPlugin {
    /// What the code itself says it is.
    pub identity: PluginIdentity,
    /// What it declared about how it wants scheduling.
    pub traits: DeclaredTraits,
    /// The controls, in the order the plugin declared them.
    pub params: Vec<Declaration>,
    /// The headings over them, in the order they opened.
    pub groups: Vec<GroupRun>,
    /// Everything the sink and the descriptor declined, each a line in the scan
    /// report. Every one of these answers `false` to
    /// [`LfxRejection::refuses_the_effect`]; a refusal that does not is why the
    /// plugin is absent from [`BrokerMessage::Described`] altogether.
    pub report: Vec<LfxRejection>,
}

impl DescribedPlugin {
    /// The same record, if its identity and its panel are inside the header's
    /// ceilings.
    ///
    /// The trait block has nothing to bound: it is a fixed set of numbers, and
    /// the one of them with a ceiling is held by
    /// [`DeclaredTraits::temporal_window`]. **Everything else here does**, and
    /// the count of rows is not enough of it. The sink asks every one of these
    /// numbers where a stranger's bytes arrive - but the sink runs in the
    /// broker, which is the process holding the stranger's compiled code, so a
    /// broker that plugin has corrupted can put a declaration with an eight
    /// kilobyte label or a dropdown of a hundred thousand options on the wire
    /// and nothing between it and the Addons page would be smaller than the
    /// transport's own 8 MiB. So every string and every list inside a
    /// declaration is asked again here, off the pipe, before anything is kept -
    /// which is what §3.2 means by the broker's direction being held to the
    /// header's ceilings as it comes off.
    ///
    /// It matters more than the equivalent check on a manifest, because the
    /// declarations are what `schema::lower` leaks for the session: a rescan
    /// re-describes, so an unbounded string kept here is an unbounded string
    /// leaked once per scan.
    ///
    /// # Errors
    ///
    /// [`LfxRejection::PastCeiling`], as [`PluginIdentity::checked`], naming
    /// which ceiling and how far past it; or [`LfxRejection::TooManyParams`]
    /// for a panel past the header's own ceiling on rows, which is the refusal
    /// the sink itself gives the same number.
    pub fn checked(self) -> Result<Self, LfxRejection> {
        let declared = u32::try_from(self.params.len()).unwrap_or(u32::MAX);
        if declared > lumit_lfx_abi::LFX_MAX_PARAMS {
            return Err(LfxRejection::TooManyParams { declared });
        }
        // A heading over every row is the most a panel can hold, and a line or
        // two per row is the most one plugin's report can - so the ceiling on
        // rows is the number that bounds both.
        bounded_count(Ceiling::Params, subject::PANEL, self.groups.len())?;
        bounded_count(Ceiling::Params, subject::PLUGIN_REPORT, self.report.len())?;
        for declaration in &self.params {
            bounded_declaration(declaration)?;
        }
        for group in &self.groups {
            bounded_string(subject::HEADING_ID, &group.id)?;
            bounded_string(subject::HEADING_LABEL, &group.label)?;
        }
        Ok(Self {
            identity: self.identity.checked()?,
            ..self
        })
    }
}

/// One declaration, if every string and every list it carries is inside the
/// ceilings `lumit-lfx-abi` declares.
///
/// The same numbers the sink asks at the push - `LFX_MAX_STRING_BYTES`,
/// `LFX_MAX_OPTIONS`, `LFX_MAX_DIVIDERS`, `LFX_MAX_FILTERS`,
/// `LFX_MAX_CURVE_POINTS` - asked a second time where the record comes off the
/// pipe. Twice on purpose: the sink runs in the broker and the broker is not
/// trusted, so "the two sides agree by construction" is only true while both
/// sides ask.
fn bounded_declaration(declaration: &Declaration) -> Result<(), LfxRejection> {
    bounded_string(subject::CONTROL_ID, &declaration.id)?;
    bounded_string(subject::CONTROL_LABEL, &declaration.label)?;
    match &declaration.kind {
        Declared::Choice {
            options,
            dividers_after,
            ..
        } => {
            bounded_count(Ceiling::Options, subject::DROPDOWN, options.len())?;
            bounded_count(Ceiling::Dividers, subject::DROPDOWN, dividers_after.len())?;
            for option in options {
                bounded_string(subject::DROPDOWN_OPTION, option)?;
            }
        }
        Declared::File {
            filter,
            filter_name,
        } => {
            bounded_count(Ceiling::Filters, subject::FILE_CONTROL, filter.len())?;
            bounded_string(subject::FILE_FILTER, filter_name)?;
            for extension in filter {
                bounded_string(subject::FILE_FILTER, extension)?;
            }
        }
        Declared::Curve { default } => {
            bounded_count(Ceiling::CurvePoints, subject::TONE_CURVE, default.len())?;
        }
        // Every other kind is numbers and a flag word, which the wire's own
        // types bound: there is nothing here a stranger can make long.
        Declared::Float { .. }
        | Declared::Slider { .. }
        | Declared::Int { .. }
        | Declared::Angle { .. }
        | Declared::Bool { .. }
        | Declared::Colour { .. }
        | Declared::Seed
        | Declared::Point2 { .. }
        | Declared::Point3 { .. }
        | Declared::Action => {}
    }
    Ok(())
}

impl From<DescribedPlugin> for PluginDescriptor {
    /// The shape the lowering reads, out of the shape the pipe carries.
    ///
    /// The two identities are the same plugin seen from either side of the
    /// boundary - [`PluginIdentity`] carries the ABI version and the required
    /// list a negotiation needs, and [`crate::describe::Identity`] carries the
    /// trait block a schema needs - so this is where the wire's raw trait
    /// numbers become the owned mirror [`crate::schema::traits_of`] lowers.
    /// **Nothing may grow a third shape**: a `Described` meets the lowering
    /// here and nowhere else.
    fn from(described: DescribedPlugin) -> Self {
        let identity = described.identity;
        Self {
            identity: crate::describe::Identity {
                id: identity.id,
                name: identity.name,
                vendor: identity.vendor,
                major: identity.major,
                minor: identity.minor,
                patch: identity.patch,
                categories: identity.categories,
                traits: Some(described.traits.into()),
                required_extensions: identity.required_extensions,
            },
            params: described.params,
            groups: described.groups,
            report: described.report,
        }
    }
}

impl From<DeclaredTraits> for crate::describe::Traits {
    /// The wire's eight numbers as the owned mirror, field for field.
    ///
    /// No defaulting happens here and none may: every zero in a trait block is
    /// a declaration meaning "unstated", and turning an unstated field into the
    /// pessimistic answer is [`crate::schema::traits_of`]'s job, written down
    /// once (docs/impl/lfx.md §2.4). A conversion that helpfully filled one in
    /// would make a `memset` block schedule as trivial.
    fn from(declared: DeclaredTraits) -> Self {
        Self {
            cost: declared.cost,
            roi_kind: declared.roi_kind,
            roi_padding_px: declared.roi_padding_px,
            temporal_lo: declared.temporal_lo,
            temporal_hi: declared.temporal_hi,
            alpha: declared.alpha,
            flags: declared.flags,
            scratch_bytes_per_megapixel: declared.scratch_bytes_per_megapixel,
        }
    }
}

impl From<&crate::describe::Traits> for DeclaredTraits {
    /// And back, for the side that read the trait block off the frozen struct.
    fn from(declared: &crate::describe::Traits) -> Self {
        Self {
            cost: declared.cost,
            roi_kind: declared.roi_kind,
            roi_padding_px: declared.roi_padding_px,
            temporal_lo: declared.temporal_lo,
            temporal_hi: declared.temporal_hi,
            alpha: declared.alpha,
            flags: declared.flags,
            scratch_bytes_per_megapixel: declared.scratch_bytes_per_megapixel,
        }
    }
}

impl From<PluginDescriptor> for DescribedPlugin {
    /// One described plugin, as it goes on the wire.
    ///
    /// The broker's direction: the module was opened there, the sink filled in
    /// there, and this is the record that crosses. A descriptor that declared
    /// no trait block crosses as the zeroed one, which is the same declaration -
    /// every field unstated - and lowers back to the pessimistic case at the
    /// other end.
    ///
    /// The ABI version is this build's rather than a field read off the
    /// descriptor, and that is not a shortcut: a module declaring any other
    /// number never opens at all, so by the time a descriptor exists to convert
    /// there is only one number it can have.
    fn from(plugin: PluginDescriptor) -> Self {
        let identity = plugin.identity;
        Self {
            traits: identity
                .traits
                .as_ref()
                .map(DeclaredTraits::from)
                .unwrap_or_default(),
            identity: PluginIdentity {
                id: identity.id,
                name: identity.name,
                vendor: identity.vendor,
                major: identity.major,
                minor: identity.minor,
                patch: identity.patch,
                categories: identity.categories,
                abi_version: lumit_lfx_abi::LFX_ABI_VERSION,
                required_extensions: identity.required_extensions,
            },
            params: plugin.params,
            groups: plugin.groups,
            report: plugin.report,
        }
    }
}

/// One frame to render, as the host asks for it.
///
/// The host-side half of `lfx_process`. The plugin's array of values is not
/// here, because the host sent it with [`HostMessage::Values`] and the broker
/// holds it: a render message that carried the values again would be the
/// biggest thing on the control plane and would say nothing new.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProcessRequest {
    /// The comp frame being rendered.
    pub time: f64,
    /// The depth of every picture in this request, input and output alike.
    pub depth: PixelDepth,
    /// The output region asked for.
    pub roi: RectI,
    /// Where the input has pixels at all.
    pub dod: RectI,
    /// The picture to read, already in the ring.
    pub input: FrameRef,
    /// The frames either side of it that the declared window admits, shipped
    /// with the request rather than fetched after it. Empty for a plugin that
    /// declared no window, however loudly `lfx.temporal` asks.
    pub neighbours: Vec<FrameRef>,
    /// The slot the answer goes in.
    pub output: Slot,
}

/// What level the broker's note was filed at - the host's own ladder, not a
/// string.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum NoteKind {
    /// Something a user would want reported.
    Error,
    /// Something that worked and should not have.
    Warn,
    /// Ordinary.
    Info,
    /// A developer's.
    Debug,
    /// A developer's, by the thousand.
    Trace,
}

impl NoteKind {
    /// The `lfx_log_level` this kind is.
    #[must_use]
    pub const fn as_log_level(self) -> LfxLogLevel {
        match self {
            NoteKind::Error => LFX_LOG_ERROR,
            NoteKind::Warn => LFX_LOG_WARN,
            NoteKind::Info => LFX_LOG_INFO,
            NoteKind::Debug => LFX_LOG_DEBUG,
            NoteKind::Trace => LFX_LOG_TRACE,
        }
    }

    /// The kind a level names, or `None` for `LFX_LOG_UNSET` and for a level a
    /// newer header has and this build has not. What to do with a level nobody
    /// recognises is the broker's decision, so it is not taken here.
    #[must_use]
    pub fn from_log_level(level: LfxLogLevel) -> Option<Self> {
        match level {
            LFX_LOG_ERROR => Some(NoteKind::Error),
            LFX_LOG_WARN => Some(NoteKind::Warn),
            LFX_LOG_INFO => Some(NoteKind::Info),
            LFX_LOG_DEBUG => Some(NoteKind::Debug),
            LFX_LOG_TRACE => Some(NoteKind::Trace),
            _ => None,
        }
    }
}

/// Which entry point a [`BrokerMessage::Failed`] is about.
///
/// A name from a closed list rather than a free string, so the strike
/// machinery, the refusal table and the badge all read the same word for the
/// same failure (docs/14 §3). `Failed` is the answer at every entry point,
/// including one reached with a handle that is rubbish - never "unsupported",
/// which would tell a plugin the feature is missing when the truth is its
/// handle is (docs/impl/lfx.md §3.5).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum HostAction {
    /// Reading the bundle's manifest, before any of its code ran.
    Manifest,
    /// Opening the module and describing what is in it.
    Describe,
    /// Making an instance.
    CreateInstance,
    /// Putting new values into one.
    Values,
    /// Pressing one of its buttons.
    Action,
    /// Rendering a frame.
    Process,
    /// Destroying an instance.
    Destroy,
    /// Mapping the ring the host named.
    OpenRing,
}

/// What the host says.
///
/// No `PartialEq`: a [`lumit_peer::Proof`] deliberately has none, so that the
/// only comparison anybody can write for one is the constant-time check the
/// crate hands out.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum HostMessage {
    /// The host's half of the handshake: a nonce for the broker to answer, and
    /// the host's own answer to the nonce the broker opened with.
    ///
    /// The broker checks the proof **before it opens anything**. Until this
    /// arrives nothing on the far end of the pipe has been told anything, and
    /// no stranger's code has run.
    Challenge {
        /// For the broker to answer.
        nonce: lumit_peer::Nonce,
        /// The host's answer to [`BrokerMessage::Ready`]'s nonce.
        proof: lumit_peer::Proof,
    },
    /// Here is the ring; map it.
    Open {
        /// The ring's layout.
        ring: RingSpec,
    },
    /// Read this bundle's manifest - and only its manifest. The module is not
    /// opened to answer this (docs/impl/lfx.md §3.3).
    Manifest {
        /// The bundle directory, whose `Contents/lfx.toml` is the listing.
        path: String,
    },
    /// Open the module and describe what is in it, skipping these.
    ///
    /// The disable list travels with the message rather than being consulted
    /// after it, which is what makes a switched-off plugin's `init` never run -
    /// the first of the three places a disable reaches (docs/impl/lfx.md §5.4).
    Describe {
        /// Plugin identifiers the user has switched off.
        disabled: BTreeSet<String>,
    },
    /// Make an instance of one of them, with these values in its controls.
    CreateInstance {
        /// The identifier the host will use for it from now on.
        instance: InstanceId,
        /// Which plugin, **by the reverse-DNS id its descriptor declared** -
        /// the same string [`PluginIdentity::id`] carries and `lfx:` is
        /// prefixed to.
        ///
        /// The identity rather than the position, because
        /// [`HostMessage::Describe`] carries the disable list: the shape of
        /// [`BrokerMessage::Described`] depends on what the user had switched
        /// off when it was asked, and a restart is a *replay* of host-held
        /// records (docs/impl/lfx.md §3.5). A bundle describing `[A, B, C]`,
        /// an instance of `B`, then `A` switched off mid-session, then a crash:
        /// the replay re-describes to `[B, C]`, and an index of one would name
        /// `C` while the layer went on holding `B`'s values and `B`'s frame
        /// key. An id names the same plugin whatever the list has lost, and an
        /// id the broker has not got is a
        /// [`BrokerMessage::Failed`] rather than a wrong effect.
        plugin: String,
        /// Every value, in declaration order.
        values: Vec<ParamValue>,
    },
    /// Replace an instance's values. The host owns them and the plugin holds
    /// none of its own, which is what makes a restart a replay rather than a
    /// recovery (docs/impl/lfx.md §3.5).
    Values {
        /// Which instance.
        instance: InstanceId,
        /// Every value, in declaration order.
        values: Vec<ParamValue>,
    },
    /// One of an instance's `ACTION` rows was pressed.
    Action {
        /// Which instance.
        instance: InstanceId,
        /// Which row, by the identifier it declared.
        param: String,
    },
    /// Render one frame.
    Process {
        /// Which instance.
        instance: InstanceId,
        /// What to render, and where the pictures are.
        request: ProcessRequest,
    },
    /// The answer to [`BrokerMessage::NeedFrames`] - **one shipment**, however
    /// many frames were asked for.
    Frames {
        /// Every frame that was asked for, in the ring.
        frames: Vec<FrameRef>,
    },
    /// Destroy an instance.
    Destroy {
        /// Which one.
        instance: InstanceId,
    },
    /// Unload and exit.
    Shutdown,
}

impl HostMessage {
    /// Whether this message is answered.
    ///
    /// **Every message is, bar two**, and the two are not an oversight:
    /// [`HostMessage::Frames`] is the answer to a question the broker asked
    /// inside an exchange that is still running, and answering the answer would
    /// leave that exchange holding a reply nobody was waiting for; and after
    /// [`HostMessage::Shutdown`] there is nobody left to answer. Everything
    /// else has a reply, and a deadline that passes without one is a strike
    /// (docs/impl/lfx.md §3.2, §3.5).
    ///
    /// Exhaustive, with no `_` arm, so a message added to the vocabulary has to
    /// say which it is.
    #[must_use]
    pub const fn expects_reply(&self) -> bool {
        match self {
            HostMessage::Frames { .. } | HostMessage::Shutdown => false,
            HostMessage::Challenge { .. }
            | HostMessage::Open { .. }
            | HostMessage::Manifest { .. }
            | HostMessage::Describe { .. }
            | HostMessage::CreateInstance { .. }
            | HostMessage::Values { .. }
            | HostMessage::Action { .. }
            | HostMessage::Process { .. }
            | HostMessage::Destroy { .. } => true,
        }
    }

    /// The answers this message admits, by [`BrokerMessage::name`].
    ///
    /// The companion to [`HostMessage::expects_reply`], and it exists for the
    /// same reason: a rule about the conversation belongs on the message rather
    /// than in a list somebody keeps. `expects_reply` says *whether* an answer
    /// is waited on; this says **which**, so the supervisor can hold what
    /// arrived to the question it asked instead of taking whatever came.
    ///
    /// A reply the question does not admit is worse than a refusal. It means
    /// the two ends have fallen out of step: the answer that was really meant
    /// for this message is still on the pipe, and the next message will collect
    /// it. So an answer out of turn is a strike that replaces the broker, and
    /// counting it as a success - which is what a supervisor that only checked
    /// for `Failed` would do - would put the strike count back to nought over
    /// and over and leave "three *consecutive* strikes" unreachable
    /// (docs/impl/lfx.md §3.5).
    ///
    /// **Where the lists are held to.** The seven a render or a control action
    /// sends are held by the supervisor's `action`, which asks this of every
    /// message it waits on. The two the handshake sends are held at the
    /// handshake: `Open`'s by `ring_is_shared`, which reads this very list, and
    /// `Challenge`'s by the exhaustive match that takes a `Hello` and refuses
    /// everything else by name - one admissible answer, so the match and the
    /// list say the same thing, and a second name written here would have to be
    /// written there as well.
    ///
    /// [`BrokerMessage::Failed`] is not in any of these lists. It answers
    /// every message and it is read before this one is asked, because it is a
    /// strike with a sentence rather than an answer out of turn. The two the
    /// exchange loop consumes without returning are not here either, for the
    /// reason [`BrokerMessage::is_interim`] gives.
    ///
    /// Exhaustive, with no `_` arm, so a message added to the vocabulary has to
    /// say what answers it.
    #[must_use]
    pub const fn answers(&self) -> &'static [&'static str] {
        match self {
            HostMessage::Challenge { .. } => &["Hello"],
            // Two, because the broker says so either way rather than leaving
            // the host to wait out the handshake timeout for a ring it could
            // not map.
            HostMessage::Open { .. } => &["RingOpened", "RingRefused"],
            HostMessage::Manifest { .. } => &["Manifested"],
            HostMessage::Describe { .. } => &["Described"],
            HostMessage::CreateInstance { .. } => &["Created"],
            HostMessage::Values { .. }
            | HostMessage::Action { .. }
            | HostMessage::Destroy { .. } => &["Done"],
            HostMessage::Process { .. } => &["Processed"],
            // The two that are never waited on have nothing to admit.
            HostMessage::Frames { .. } | HostMessage::Shutdown => &[],
        }
    }

    /// What this message is called, for a log line and for the suite.
    ///
    /// Exhaustive, with no `_` arm, which is what lets a test assert that a
    /// rule said to hold over "every message" holds over every *variant* of the
    /// enum rather than over the ones somebody remembered to put in a list: a
    /// variant added tomorrow fails this match, and the names it does not have
    /// fail the comparison beside it.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            HostMessage::Challenge { .. } => "Challenge",
            HostMessage::Open { .. } => "Open",
            HostMessage::Manifest { .. } => "Manifest",
            HostMessage::Describe { .. } => "Describe",
            HostMessage::CreateInstance { .. } => "CreateInstance",
            HostMessage::Values { .. } => "Values",
            HostMessage::Action { .. } => "Action",
            HostMessage::Process { .. } => "Process",
            HostMessage::Frames { .. } => "Frames",
            HostMessage::Destroy { .. } => "Destroy",
            HostMessage::Shutdown => "Shutdown",
        }
    }
}

/// What the broker says.
///
/// No `PartialEq`, for the reason [`HostMessage`] has none.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum BrokerMessage {
    /// The actual first word: a nonce, and nothing else.
    ///
    /// Nothing is proved here and nothing is revealed - whoever connected to
    /// the endpoint gets to say this much. The host answers with a
    /// [`HostMessage::Challenge`] carrying a proof only the real host can
    /// compute.
    Ready {
        /// For the host to answer.
        nonce: lumit_peer::Nonce,
    },
    /// The broker's answer to the host's challenge, with the protocol it
    /// speaks. The host checks the **proof before the version**, so an impostor
    /// is not told which build it faces.
    Hello {
        /// The protocol the broker speaks.
        version: u32,
        /// The broker's answer to [`HostMessage::Challenge`]'s nonce.
        proof: lumit_peer::Proof,
    },
    /// The ring is mapped, and the host may now unlink its path.
    RingOpened,
    /// The broker could not map the ring the host named. Sent so the host
    /// returns at once rather than waiting out the handshake timeout for an
    /// acknowledgement that is never coming.
    RingRefused,
    /// What the bundle's manifest lists - read without the module being opened.
    Manifested {
        /// One per plugin the manifest declares.
        entries: Vec<PluginIdentity>,
    },
    /// What the module actually holds.
    ///
    /// **The list's membership depends on the disable set** the describe
    /// carried, so nothing may name a plugin by its position in it:
    /// [`HostMessage::CreateInstance`] names the descriptor's own id, and the
    /// broker looks that up in the table this answer built.
    Described {
        /// One per plugin that described itself successfully.
        plugins: Vec<DescribedPlugin>,
        /// One per plugin the describe **turned away**, by its own id, with the
        /// refusal that turned it away.
        ///
        /// A plugin dropped here would otherwise reach the host as an absence
        /// and nothing else - not catalogued, not refused, not in the report -
        /// which is indistinguishable from one the user switched off and leaves
        /// §5.3's `REFUSED` table, whose whole content is "what `offer()`
        /// turned away this session, with its own sentence", with no sentence
        /// to carry. The describe runs in the second process, so this is the
        /// only road those sentences have.
        refused: Vec<(String, LfxRejection)>,
        /// Lines about the **bundle** rather than about any one plugin in it -
        /// a descriptor with no readable id, a descriptor shorter than this
        /// header, an id two descriptors both declare.
        ///
        /// They have nowhere else to go: every other report line belongs to a
        /// plugin and rides in that plugin's own record, and these belong to
        /// the file. Filing them against the first plugin in the list would say
        /// something untrue about that plugin, and dropping them would leave the
        /// Addons page unable to say why a bundle holds eleven effects where
        /// its author wrote twelve.
        report: Vec<LfxRejection>,
    },
    /// The instance exists.
    Created,
    /// The frame is in the ring.
    Processed {
        /// Where.
        slot: Slot,
        /// What the instance asked `lfx.temporal` for, as offsets relative to
        /// the frame that was rendered. The host reads them into the neighbour
        /// window the next frame key is taken over.
        frames_needed: Vec<i32>,
    },
    /// The message was carried out and there is nothing to say about it.
    ///
    /// The plain acknowledgement, and **not optional**:
    /// [`HostMessage::Values`], [`HostMessage::Action`] and
    /// [`HostMessage::Destroy`] have no other answer, and without it each would
    /// be a guaranteed control-deadline timeout that also restarts the broker.
    Done,
    /// The plugin wants frames the host has not sent. Answered with exactly one
    /// [`HostMessage::Frames`], which is the point of asking for the lot at
    /// once.
    NeedFrames {
        /// Every frame, in one list.
        frames: Vec<FrameWanted>,
    },
    /// The plugin said something through the host's log. Nothing here is modal.
    Note {
        /// What level it was filed at.
        kind: NoteKind,
        /// What it said.
        text: String,
    },
    /// Something went wrong, as a sentence rather than a status code: the host
    /// puts it on a badge and the user reads it.
    Failed {
        /// Which entry point.
        action: HostAction,
        /// What went wrong.
        message: String,
    },
}

impl BrokerMessage {
    /// Whether this is something the exchange loop consumes and goes on
    /// waiting, rather than the reply it is waiting for.
    ///
    /// Two are: the broker asking for frames, and the plugin saying something.
    /// Both can arrive any number of times in the middle of one action, which
    /// is why neither may be mistaken for its answer - a `Note` counted as a
    /// reply would end the wait with the action still running.
    ///
    /// Exhaustive, with no `_` arm.
    #[must_use]
    pub const fn is_interim(&self) -> bool {
        match self {
            BrokerMessage::NeedFrames { .. } | BrokerMessage::Note { .. } => true,
            BrokerMessage::Ready { .. }
            | BrokerMessage::Hello { .. }
            | BrokerMessage::RingOpened
            | BrokerMessage::RingRefused
            | BrokerMessage::Manifested { .. }
            | BrokerMessage::Described { .. }
            | BrokerMessage::Created
            | BrokerMessage::Processed { .. }
            | BrokerMessage::Done
            | BrokerMessage::Failed { .. } => false,
        }
    }

    /// What this message is called, for a log line and for the suite.
    ///
    /// Exhaustive, for the reason [`HostMessage::name`] is.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            BrokerMessage::Ready { .. } => "Ready",
            BrokerMessage::Hello { .. } => "Hello",
            BrokerMessage::RingOpened => "RingOpened",
            BrokerMessage::RingRefused => "RingRefused",
            BrokerMessage::Manifested { .. } => "Manifested",
            BrokerMessage::Described { .. } => "Described",
            BrokerMessage::Created => "Created",
            BrokerMessage::Processed { .. } => "Processed",
            BrokerMessage::Done => "Done",
            BrokerMessage::NeedFrames { .. } => "NeedFrames",
            BrokerMessage::Note { .. } => "Note",
            BrokerMessage::Failed { .. } => "Failed",
        }
    }

    /// The same message, held to the ceilings the frozen header declares.
    ///
    /// # In plain terms
    ///
    /// **This is the direction the stranger talks in.** Everything in a
    /// [`BrokerMessage`] was written by somebody else's compiled code, and the
    /// only thing bounding it before this is `lumit-ipc`'s 8 MiB cap on one
    /// message - which is a manifest of a hundred thousand plugins, or a log
    /// line of eight million characters that §3.5's `MAX_NOTES` would then keep
    /// sixty-four of. `lumit-lfx-abi` declares a number for each of those
    /// things, and this is where the host's side of the pipe reads them
    /// (docs/impl/lfx.md §12).
    ///
    /// Three answers, by the shape of the thing:
    ///
    /// * **a count past a ceiling is a refusal** - a bundle with more plugins
    ///   than the header admits, a plugin with more categories or more required
    ///   extensions, a name longer than a declared string may be. None of these
    ///   can be trimmed into something true;
    /// * **a log line past `LFX_MAX_LOG_BYTES` is cut**, on a character
    ///   boundary, because a plugin that says too much should still be heard;
    /// * **the frames a plugin asks for are held to the window**
    ///   `LFX_MAX_TEMPORAL_WINDOW` admits, exactly as
    ///   [`DeclaredTraits::temporal_window`] holds the declaration itself, and
    ///   the same offset asked for twice is one frame - which bounds the list
    ///   by construction at the 129 offsets that window has.
    ///
    /// The refusal proper - the badge, the strike, the report line - is
    /// [`crate::ipc::broker`]'s; what is here is the number and the name of it.
    ///
    /// # Errors
    ///
    /// [`LfxRejection::PastCeiling`] for a count or a string past the header's
    /// own number for it.
    pub fn checked(self) -> Result<Self, LfxRejection> {
        Ok(match self {
            BrokerMessage::Manifested { entries } => {
                bounded_count(Ceiling::EffectsPerBundle, subject::MANIFEST, entries.len())?;
                BrokerMessage::Manifested {
                    entries: entries
                        .into_iter()
                        .map(PluginIdentity::checked)
                        .collect::<Result<Vec<_>, _>>()?,
                }
            }
            BrokerMessage::Described {
                plugins,
                refused,
                report,
            } => {
                bounded_count(Ceiling::EffectsPerBundle, subject::MODULE, plugins.len())?;
                // A refusal is one per plugin, so the bundle's own effect
                // ceiling bounds the turned-away list exactly as it bounds the
                // kept one.
                bounded_count(Ceiling::EffectsPerBundle, subject::MODULE, refused.len())?;
                // A line about the bundle is at most one or two per descriptor,
                // so the bundle's own effect ceiling is the number that bounds
                // them too - and it is asked here for the reason every other
                // count is: the list was written in a process holding a
                // stranger's compiled code.
                bounded_count(Ceiling::EffectsPerBundle, subject::MODULE, report.len())?;
                for (id, _) in &refused {
                    bounded_string(subject::PLUGIN_ID, id)?;
                }
                BrokerMessage::Described {
                    plugins: plugins
                        .into_iter()
                        .map(DescribedPlugin::checked)
                        .collect::<Result<Vec<_>, _>>()?,
                    refused,
                    report,
                }
            }
            BrokerMessage::Note { kind, text } => BrokerMessage::Note {
                kind,
                text: cut_to_bytes(text, lumit_lfx_abi::LFX_MAX_LOG_BYTES),
            },
            // A badge sentence is a log line by another name, and it is read by
            // the same stranger's code, so it is held to the same number.
            BrokerMessage::Failed { action, message } => BrokerMessage::Failed {
                action,
                message: cut_to_bytes(message, lumit_lfx_abi::LFX_MAX_LOG_BYTES),
            },
            BrokerMessage::Processed {
                slot,
                frames_needed,
            } => BrokerMessage::Processed {
                slot,
                frames_needed: held_to_the_window(frames_needed),
            },
            settled @ (BrokerMessage::Ready { .. }
            | BrokerMessage::Hello { .. }
            | BrokerMessage::RingOpened
            | BrokerMessage::RingRefused
            | BrokerMessage::Created
            | BrokerMessage::Done
            | BrokerMessage::NeedFrames { .. }) => settled,
        })
    }
}

/// A string no longer than the header admits, or the refusal that names it.
fn bounded_string(subject: &'static str, value: &str) -> Result<(), LfxRejection> {
    let given = u64::try_from(value.len()).unwrap_or(u64::MAX);
    if given > Ceiling::StringBytes.limit() {
        return Err(LfxRejection::PastCeiling {
            ceiling: Ceiling::StringBytes,
            subject,
            given,
        });
    }
    Ok(())
}

/// A list no longer than the header admits, or the refusal that names it.
fn bounded_count(
    ceiling: Ceiling,
    subject: &'static str,
    given: usize,
) -> Result<(), LfxRejection> {
    let given = u64::try_from(given).unwrap_or(u64::MAX);
    if given > ceiling.limit() {
        return Err(LfxRejection::PastCeiling {
            ceiling,
            subject,
            given,
        });
    }
    Ok(())
}

/// The first `bytes` bytes of a string, cut on a character boundary.
///
/// Cut rather than refused, and never in the middle of a character: a `String`
/// that is not UTF-8 is not a `String`, and a plugin that logged a paragraph
/// has still said something worth reading the front of.
fn cut_to_bytes(text: String, bytes: u32) -> String {
    let limit = usize::try_from(bytes).unwrap_or(usize::MAX);
    if text.len() <= limit {
        return text;
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.get(..end).unwrap_or_default().to_owned()
}

/// The offsets a plugin asked for, held to the window the header admits.
///
/// Sorted and deduplicated as well as clamped, which is what bounds the list:
/// the same neighbour asked for twice is one frame, so what comes back is at
/// most the 129 offsets `LFX_MAX_TEMPORAL_WINDOW` has either side of nought. A
/// canonical order is what the frame key wants anyway - the key is over the set
/// of frames read, not over the order they were named in.
fn held_to_the_window(frames: Vec<i32>) -> Vec<i32> {
    let limit = lumit_lfx_abi::LFX_MAX_TEMPORAL_WINDOW;
    let mut held: Vec<i32> = frames
        .into_iter()
        .map(|offset| offset.clamp(-limit, limit))
        .collect();
    held.sort_unstable();
    held.dedup();
    held
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Every one of the eleven host messages, so a rule asserted over "every
    /// message" is asserted over all of them rather than over the two somebody
    /// remembered.
    ///
    /// A hand-written list is the thing that drifts from the enum beside it,
    /// which is why [`the_message_lists_hold_every_variant_of_their_enum`] pins
    /// this one to [`HostMessage::name`]'s exhaustive match rather than to a
    /// count.
    fn every_host_message() -> Vec<HostMessage> {
        let nonce = lumit_peer::Nonce::generate().expect("a nonce");
        let secret = lumit_peer::Secret::generate().expect("a secret");
        let proof = lumit_peer::Proof::host(&secret, nonce);
        vec![
            HostMessage::Challenge { nonce, proof },
            HostMessage::Open {
                ring: RingSpec {
                    path: "ring".into(),
                    slots: 3,
                    slot_bytes: 64,
                    depth: PixelDepth::F16,
                },
            },
            HostMessage::Manifest {
                path: "Bundle.lfx.bundle".into(),
            },
            HostMessage::Describe {
                disabled: BTreeSet::new(),
            },
            HostMessage::CreateInstance {
                instance: 1,
                plugin: "com.example.blur".into(),
                values: vec![ParamValue::Float(1.0)],
            },
            HostMessage::Values {
                instance: 1,
                values: Vec::new(),
            },
            HostMessage::Action {
                instance: 1,
                param: "reset".into(),
            },
            HostMessage::Process {
                instance: 1,
                request: ProcessRequest {
                    time: 0.0,
                    depth: PixelDepth::F32,
                    roi: RectI::of(8, 8),
                    dod: RectI::of(8, 8),
                    input: FrameRef { time: 0.0, slot: 0 },
                    neighbours: Vec::new(),
                    output: 1,
                },
            },
            HostMessage::Frames { frames: Vec::new() },
            HostMessage::Destroy { instance: 1 },
            HostMessage::Shutdown,
        ]
    }

    /// One plugin identity inside every ceiling, so that a test about one
    /// ceiling is about that ceiling alone.
    fn identity(id: &str) -> PluginIdentity {
        PluginIdentity {
            id: id.into(),
            name: "Blur".into(),
            vendor: "Example".into(),
            major: 1,
            minor: 2,
            patch: 3,
            categories: vec![lumit_lfx_abi::LFX_CATEGORY_BLUR_SHARPEN],
            abi_version: lumit_lfx_abi::LFX_ABI_VERSION,
            required_extensions: vec!["lfx.temporal".into()],
        }
    }

    /// Every one of the twelve broker messages, for the same reason
    /// [`every_host_message`] holds every host one - and for one more: this is
    /// the direction a stranger talks in, so the handshake pair, the frame
    /// requests and the note are precisely the messages a serde or a `bincode`
    /// change would otherwise break at run time in the broker rather than
    /// here.
    fn every_broker_message() -> Vec<BrokerMessage> {
        let nonce = lumit_peer::Nonce::generate().expect("a nonce");
        let secret = lumit_peer::Secret::generate().expect("a secret");
        vec![
            BrokerMessage::Ready { nonce },
            BrokerMessage::Hello {
                version: PROTOCOL_VERSION,
                proof: lumit_peer::Proof::broker(&secret, nonce),
            },
            BrokerMessage::RingOpened,
            BrokerMessage::RingRefused,
            BrokerMessage::Manifested {
                entries: vec![identity("com.example.blur")],
            },
            BrokerMessage::Described {
                plugins: vec![DescribedPlugin {
                    identity: identity("com.example.blur"),
                    traits: DeclaredTraits {
                        cost: lumit_lfx_abi::LFX_COST_HEAVY,
                        temporal_lo: -2,
                        temporal_hi: 2,
                        ..DeclaredTraits::default()
                    },
                    ..DescribedPlugin::default()
                }],
                refused: vec![(
                    "com.example.broken".into(),
                    LfxRejection::DescribeRefused {
                        id: "com.example.broken".into(),
                    },
                )],
                report: Vec::new(),
            },
            BrokerMessage::Created,
            BrokerMessage::Processed {
                slot: 3,
                frames_needed: vec![-1, 0, 1],
            },
            BrokerMessage::Done,
            BrokerMessage::NeedFrames {
                frames: vec![FrameWanted { time: 11.0 }, FrameWanted { time: 13.0 }],
            },
            BrokerMessage::Note {
                kind: NoteKind::Warn,
                text: "a sentence".into(),
            },
            BrokerMessage::Failed {
                action: HostAction::Describe,
                message: "no".into(),
            },
        ]
    }

    /// The two helpers above are hand-written lists, and a hand-written list is
    /// the thing that drifts from the enum beside it - a number in a comment
    /// that the list below it does not have is how it starts. So the names are
    /// pinned in three places at once: `name()` is an exhaustive match with no
    /// `_` arm, the arrays here have a length, and the lists must cover them. A
    /// variant added tomorrow fails the match, the length or the comparison,
    /// and a rule asserted over "every message" cannot quietly come to be
    /// asserted over all but one.
    #[test]
    fn the_message_lists_hold_every_variant_of_their_enum() {
        const HOST_MESSAGE_NAMES: [&str; 11] = [
            "Challenge",
            "Open",
            "Manifest",
            "Describe",
            "CreateInstance",
            "Values",
            "Action",
            "Process",
            "Frames",
            "Destroy",
            "Shutdown",
        ];
        const BROKER_MESSAGE_NAMES: [&str; 12] = [
            "Ready",
            "Hello",
            "RingOpened",
            "RingRefused",
            "Manifested",
            "Described",
            "Created",
            "Processed",
            "Done",
            "NeedFrames",
            "Note",
            "Failed",
        ];

        let hosts = every_host_message();
        assert_eq!(
            hosts.len(),
            HOST_MESSAGE_NAMES.len(),
            "the list says one host message twice, or leaves one out"
        );
        assert_eq!(
            hosts.iter().map(HostMessage::name).collect::<BTreeSet<_>>(),
            HOST_MESSAGE_NAMES.into_iter().collect::<BTreeSet<_>>(),
            "every host message is in the list the rules are asserted over"
        );

        let brokers = every_broker_message();
        assert_eq!(
            brokers.len(),
            BROKER_MESSAGE_NAMES.len(),
            "the list says one broker message twice, or leaves one out"
        );
        assert_eq!(
            brokers
                .iter()
                .map(BrokerMessage::name)
                .collect::<BTreeSet<_>>(),
            BROKER_MESSAGE_NAMES.into_iter().collect::<BTreeSet<_>>(),
            "every broker message is in the list the rules are asserted over"
        );
    }

    /// The rule the strike machinery rests on: a deadline with no reply is a
    /// strike, so every message must have a reply to be missing. The two
    /// exceptions are exactly the one consumed inside the exchange loop and the
    /// one sent when there is nobody left to answer - named here so that a
    /// third exception has to be argued for rather than added
    /// (docs/impl/lfx.md §3.2, §14 item 5).
    #[test]
    fn every_host_message_but_frames_and_shutdown_expects_a_reply() {
        let unanswered: BTreeSet<&str> = every_host_message()
            .iter()
            .filter(|message| !message.expects_reply())
            .map(HostMessage::name)
            .collect();
        assert_eq!(
            unanswered,
            BTreeSet::from(["Frames", "Shutdown"]),
            "these and no others go unanswered"
        );
    }

    /// The other half of the same rule: a message that is waited on names the
    /// answer it admits, and a message that is not names none.
    ///
    /// Every name it gives is one of the broker's own, is not `Failed` - which
    /// answers everything and is read before this is asked - and is not one of
    /// the two the exchange loop consumes without returning, since an answer
    /// that ends the wait cannot also be a thing the wait goes past. It is what
    /// lets the supervisor tell an answer from a reply to somebody else's
    /// question, and a broker out of step from a broker that said no
    /// (docs/impl/lfx.md §3.5).
    #[test]
    fn every_answered_message_names_the_answer_it_admits() {
        let replies: BTreeSet<&str> = every_broker_message()
            .iter()
            .filter(|message| !message.is_interim() && message.name() != "Failed")
            .map(BrokerMessage::name)
            .collect();
        for message in every_host_message() {
            assert_eq!(
                message.expects_reply(),
                !message.answers().is_empty(),
                "{} is waited on if and only if it names an answer",
                message.name()
            );
            for answer in message.answers() {
                assert!(
                    replies.contains(answer),
                    "{} admits {answer}, which is not an answer the broker gives",
                    message.name()
                );
            }
        }
    }

    /// `Done` is the plain acknowledgement, and the three messages that have no
    /// other answer are the reason it is not optional. Drop it and each becomes
    /// a guaranteed control-deadline timeout that also restarts the broker, so
    /// a user dragging one slider three times disables the plugin.
    #[test]
    fn the_three_messages_with_nothing_to_report_are_answered_with_done() {
        for message in [
            HostMessage::Values {
                instance: 1,
                values: Vec::new(),
            },
            HostMessage::Action {
                instance: 1,
                param: "reset".into(),
            },
            HostMessage::Destroy { instance: 1 },
        ] {
            assert!(
                message.expects_reply(),
                "{message:?} is waited on, so it needs an answer"
            );
        }
        assert!(
            !BrokerMessage::Done.is_interim(),
            "Done is the reply itself, never something the loop waits past"
        );
    }

    /// The two answers that are really questions. An exchange that counted
    /// either as its reply would return with the action still running - the
    /// plugin's note ending a render, or a request for frames ending the render
    /// that was about to be given them.
    #[test]
    fn a_note_or_a_request_for_frames_is_not_the_reply() {
        let interim: BTreeSet<&str> = every_broker_message()
            .iter()
            .filter(|message| message.is_interim())
            .map(BrokerMessage::name)
            .collect();
        assert_eq!(
            interim,
            BTreeSet::from(["NeedFrames", "Note"]),
            "these and no others are consumed with the wait still running"
        );
    }

    /// The protocol is versioned apart from the ABI, which is the promise
    /// docs/12 §3.5 makes. Two constants in two crates is how it is kept, and
    /// this test reads the declaring line back to check it is still a number of
    /// this crate's own rather than the ABI's borrowed.
    ///
    /// The name says what the test does rather than what the property is,
    /// deliberately: **the two numbers being equal at one today is a
    /// coincidence**, so no assertion here can tell "they agree" from "they are
    /// the same constant", and the honest half of the check is textual. A
    /// structural half arrives the day they differ - until then, the line is
    /// what there is to read.
    #[test]
    fn the_protocol_version_is_declared_as_a_number_of_its_own() {
        assert_eq!(PROTOCOL_VERSION, 1);
        let source = include_str!("proto.rs");
        let declaration = source
            .lines()
            .map(str::trim_start)
            .find(|line| line.starts_with("pub const PROTOCOL_VERSION"))
            .expect("the protocol version is declared in this file");
        assert!(
            !declaration.contains("ABI") && !declaration.contains("abi"),
            "the protocol version must be a number of its own, not the ABI's: {declaration}"
        );
    }

    /// Every value carries the tag the header gives its kind, so the wrapper's
    /// kind check is a check against the ABI rather than against a second
    /// opinion. `LFX_PARAM_UNSET` is nobody's tag.
    #[test]
    fn every_value_names_the_kind_the_header_gives_it() {
        let values = [
            (ParamValue::Float(0.0), LFX_PARAM_FLOAT),
            (ParamValue::Slider(0.0), LFX_PARAM_SLIDER),
            (ParamValue::Angle(0.0), LFX_PARAM_ANGLE),
            (ParamValue::Int(0), LFX_PARAM_INT),
            (ParamValue::Seed(0), LFX_PARAM_SEED),
            (ParamValue::Bool(false), LFX_PARAM_BOOL),
            (ParamValue::Choice(0), LFX_PARAM_CHOICE),
            (ParamValue::Colour([0.0; 4]), LFX_PARAM_COLOUR),
            (ParamValue::Point2([0.0; 2]), LFX_PARAM_POINT2),
            (ParamValue::Point3([0.0; 3]), LFX_PARAM_POINT3),
            (ParamValue::Curve(Vec::new()), LFX_PARAM_CURVE),
            (ParamValue::File(None), LFX_PARAM_FILE),
        ];
        let mut seen = std::collections::BTreeSet::new();
        for (value, kind) in values {
            assert_eq!(
                value.kind(),
                kind,
                "{value:?} is tagged with the wrong kind"
            );
            assert_ne!(
                value.kind(),
                lumit_lfx_abi::LFX_PARAM_UNSET,
                "{value:?} crossed untagged"
            );
            assert!(seen.insert(value.kind()), "{value:?} shares a tag");
        }
    }

    /// The two depths are the header's own two, by the header's own numbers,
    /// and everything else - the unset format included - is nobody's depth.
    #[test]
    fn the_wire_depth_is_the_headers_own_pixel_format() {
        for depth in [PixelDepth::F16, PixelDepth::F32] {
            assert_eq!(
                PixelDepth::from_pixel_format(depth.as_pixel_format()),
                Some(depth)
            );
        }
        assert_eq!(PixelDepth::F16.as_pixel_format(), LFX_RGBA_F16);
        assert_eq!(PixelDepth::F32.as_pixel_format(), LFX_RGBA_F32);
        assert_eq!(
            PixelDepth::F16.bytes_per_sample() * 2,
            PixelDepth::F32.bytes_per_sample()
        );
        assert_eq!(
            PixelDepth::from_pixel_format(lumit_lfx_abi::LFX_PIXEL_UNSET),
            None,
            "an unset format is not a depth"
        );
        assert_eq!(PixelDepth::from_pixel_format(99), None);
    }

    /// The note levels are the header's log ladder, one for one, and an unset
    /// level is not one of them.
    #[test]
    fn a_note_is_filed_at_one_of_the_headers_own_levels() {
        for kind in [
            NoteKind::Error,
            NoteKind::Warn,
            NoteKind::Info,
            NoteKind::Debug,
            NoteKind::Trace,
        ] {
            assert_eq!(NoteKind::from_log_level(kind.as_log_level()), Some(kind));
        }
        assert_eq!(NoteKind::from_log_level(lumit_lfx_abi::LFX_LOG_UNSET), None);
    }

    /// **No pixels on the control plane.** A render of a 4K frame is a message
    /// of tens of bytes naming slots; the picture is already in the ring. The
    /// guard is a size, because the failure it catches - somebody adding a
    /// `Vec<f32>` to a message "just for the neighbours" - would still compile
    /// and would still work, slowly, for ever.
    #[test]
    fn no_picture_crosses_the_control_plane() {
        let request = HostMessage::Process {
            instance: 7,
            request: ProcessRequest {
                time: 12.0,
                depth: PixelDepth::F16,
                roi: RectI::of(3840, 2160),
                dod: RectI::of(3840, 2160),
                input: FrameRef {
                    time: 12.0,
                    slot: 0,
                },
                neighbours: (1..=5)
                    .map(|n| FrameRef {
                        time: 12.0 - f64::from(n),
                        slot: n,
                    })
                    .collect(),
                output: 6,
            },
        };
        let bytes = bincode::serialize(&request).expect("a render message goes on the wire");
        assert!(
            bytes.len() < 512,
            "a render message is tens of bytes, not {}: something is carrying pixels",
            bytes.len()
        );
        assert!(
            bytes.len() < lumit_ipc::MAX_MESSAGE_BYTES,
            "and it is nowhere near the transport's cap"
        );
    }

    /// The rectangle agrees with the ABI about which edge is in: a 1920 by 1080
    /// region is 1920 across, and a rectangle that runs backwards is empty
    /// rather than negative.
    #[test]
    fn a_region_counts_its_pixels_the_way_the_header_does() {
        let hd = RectI::of(1920, 1080);
        assert_eq!((hd.width(), hd.height()), (1920, 1080));
        assert_eq!(hd.samples(), 1920 * 1080 * 4);
        let backwards = RectI {
            x0: 10,
            y0: 10,
            x1: 0,
            y1: 0,
        };
        assert_eq!((backwards.width(), backwards.height()), (0, 0));
        assert_eq!(backwards.samples(), 0);
    }

    /// A zeroed trait block declares nothing, and the window it yields is the
    /// one frame being rendered - so a plugin that said nothing never sees a
    /// neighbour. A declaration past the header's ceiling is held to it, and
    /// one that does not contain the frame being rendered cannot push the
    /// window off it.
    #[test]
    fn an_unstated_window_is_the_frame_itself() {
        assert_eq!(DeclaredTraits::default().temporal_window(), (0, 0));

        let wide = DeclaredTraits {
            temporal_lo: -5,
            temporal_hi: 5,
            ..DeclaredTraits::default()
        };
        assert_eq!(wide.temporal_window(), (-5, 5));

        let absurd = DeclaredTraits {
            temporal_lo: -10_000,
            temporal_hi: 10_000,
            ..DeclaredTraits::default()
        };
        let limit = lumit_lfx_abi::LFX_MAX_TEMPORAL_WINDOW;
        assert_eq!(absurd.temporal_window(), (-limit, limit));

        let backwards = DeclaredTraits {
            temporal_lo: 3,
            temporal_hi: -3,
            ..DeclaredTraits::default()
        };
        assert_eq!(
            backwards.temporal_window(),
            (0, 0),
            "a window that excludes the frame being rendered is not one"
        );
    }

    /// Every message round-trips through the wire format the transport uses,
    /// which is the property a protocol enum has to have and the one nothing
    /// else in the crate would notice the loss of - so it is asserted over
    /// every variant of both enums rather than over a handful. The handshake
    /// pair is the first thing on the wire and the only pair carrying a
    /// [`lumit_peer::Nonce`] and a [`lumit_peer::Proof`]; without them here, a
    /// change to serde's treatment of a fixed-size array would be found by the
    /// broker at run time instead.
    #[test]
    fn every_message_round_trips_through_the_wire_format() {
        for message in every_host_message() {
            let bytes = bincode::serialize(&message).expect("serialise");
            let back: HostMessage = bincode::deserialize(&bytes).expect("deserialise");
            assert_eq!(format!("{back:?}"), format!("{message:?}"));
            assert_eq!(bincode::serialize(&back).expect("serialise"), bytes);
        }
        for message in every_broker_message() {
            let bytes = bincode::serialize(&message).expect("serialise");
            let back: BrokerMessage = bincode::deserialize(&bytes).expect("deserialise");
            assert_eq!(format!("{back:?}"), format!("{message:?}"));
            assert_eq!(bincode::serialize(&back).expect("serialise"), bytes);
        }
    }

    /// **An instance names its plugin, not its place in a list.**
    /// `Describe` carries the disable list, so `Described`'s membership depends
    /// on what the user had switched off when it was asked - and a restart is a
    /// replay of host-held records (docs/impl/lfx.md §3.5, D8). A bundle of
    /// three, an instance of the second, one switched off mid-session: an index
    /// would name a different effect on the replay while the layer went on
    /// holding the first one's values and frame key.
    #[test]
    fn an_instance_names_its_plugin_by_identity_rather_than_by_position() {
        let described = |ids: &[&str]| -> Vec<DescribedPlugin> {
            ids.iter()
                .map(|id| DescribedPlugin {
                    identity: identity(id),
                    ..DescribedPlugin::default()
                })
                .collect()
        };
        let create = HostMessage::CreateInstance {
            instance: 1,
            plugin: "com.example.b".into(),
            values: Vec::new(),
        };
        let HostMessage::CreateInstance { plugin, .. } = &create else {
            panic!("the message that was just built");
        };

        let first_pass = described(&["com.example.a", "com.example.b", "com.example.c"]);
        let before = first_pass
            .iter()
            .position(|entry| &entry.identity.id == plugin)
            .expect("the plugin the host made an instance of");
        assert_eq!(before, 1);

        // The user switches the first one off, the plugin crashes, and the
        // replay re-describes with the disable list it now has.
        let replay = described(&["com.example.b", "com.example.c"]);
        let after = replay
            .iter()
            .position(|entry| &entry.identity.id == plugin)
            .expect("the same plugin, found by the same name");
        assert_ne!(before, after, "the position moved under the instance");
        assert_eq!(
            replay.get(after).map(|entry| entry.identity.id.as_str()),
            Some("com.example.b"),
            "and the name did not"
        );

        // A plugin the broker has not got is a lookup that finds nothing, which
        // is a `Failed` rather than whatever was at that index.
        assert!(replay
            .iter()
            .all(|entry| entry.identity.id != "com.example.a"));
    }

    /// `LFX_MAX_STRING_BYTES`, on every string the identity carries. The
    /// sentence the refusal prints never quotes the string back: the refusal is
    /// *about* a declaration nobody has checked yet.
    #[test]
    fn a_declared_string_longer_than_the_header_admits_is_refused() {
        let limit = lumit_lfx_abi::LFX_MAX_STRING_BYTES as usize;
        let long = "a".repeat(limit + 1);

        let refused = identity(&long).checked().expect_err("an id nobody meant");
        assert_eq!(
            refused,
            LfxRejection::PastCeiling {
                ceiling: Ceiling::StringBytes,
                subject: "a plugin id",
                given: limit as u64 + 1,
            }
        );
        assert!(
            !refused.to_string().contains("aaaa"),
            "the refusal quoted the stranger's own string back: {refused}"
        );

        for (subject, mut plugin) in [
            ("a plugin name", identity("com.example.blur")),
            ("a plugin vendor", identity("com.example.blur")),
            ("a required extension id", identity("com.example.blur")),
        ] {
            match subject {
                "a plugin name" => plugin.name = long.clone(),
                "a plugin vendor" => plugin.vendor = long.clone(),
                _ => plugin.required_extensions = vec![long.clone()],
            }
            let refused = plugin.checked().expect_err("past the ceiling");
            assert!(
                matches!(
                    refused,
                    LfxRejection::PastCeiling {
                        ceiling: Ceiling::StringBytes,
                        subject: named,
                        ..
                    } if named == subject
                ),
                "{refused}"
            );
        }

        // And a string of exactly the ceiling is admitted: a limit a vendor has
        // shipped up against may not be narrowed.
        let mut at_the_ceiling = identity("com.example.blur");
        at_the_ceiling.name = "a".repeat(limit);
        assert!(at_the_ceiling.checked().is_ok());
    }

    /// `LFX_MAX_CATEGORIES`. The first is the heading and the rest are search
    /// keywords, so a list of ten thousand is a search index rather than a
    /// declaration.
    #[test]
    fn more_categories_than_the_header_admits_are_refused() {
        let limit = lumit_lfx_abi::LFX_MAX_CATEGORIES as usize;
        let mut plugin = identity("com.example.blur");
        plugin.categories = vec![lumit_lfx_abi::LFX_CATEGORY_UTILITY; limit + 1];
        let refused = plugin.clone().checked().expect_err("too many families");
        assert!(
            matches!(
                refused,
                LfxRejection::PastCeiling {
                    ceiling: Ceiling::Categories,
                    given,
                    ..
                } if given == limit as u64 + 1
            ),
            "{refused}"
        );

        plugin.categories.truncate(limit);
        assert!(plugin.checked().is_ok(), "exactly the ceiling is admitted");
    }

    /// `LFX_MAX_REQUIRED_EXTENSIONS`. Each one is negotiated before `create`
    /// and re-checked at the first describe, so the list is walked twice.
    #[test]
    fn more_required_extensions_than_the_header_admits_are_refused() {
        let limit = lumit_lfx_abi::LFX_MAX_REQUIRED_EXTENSIONS as usize;
        let mut plugin = identity("com.example.blur");
        plugin.required_extensions = vec!["lfx.temporal".to_owned(); limit + 1];
        let refused = plugin.clone().checked().expect_err("too many extensions");
        assert!(
            matches!(
                refused,
                LfxRejection::PastCeiling {
                    ceiling: Ceiling::RequiredExtensions,
                    ..
                }
            ),
            "{refused}"
        );

        plugin.required_extensions.truncate(limit);
        assert!(plugin.checked().is_ok(), "exactly the ceiling is admitted");
    }

    /// The ceilings inside a **declaration**, which is the payload the describe
    /// grew and the one the count of rows says nothing about.
    ///
    /// The sink asks all of these at the push - and the sink runs in the
    /// broker, the process holding the stranger's compiled code, so asking them
    /// there is asking the suspect. A broker that plugin has corrupted can put
    /// a label of eight kilobytes or a dropdown of a hundred thousand options on
    /// the wire, and until this check the only number between that and the
    /// Addons page was the transport's own 8 MiB - which is the state §3.2 says
    /// the ceilings exist to replace. It costs more than a manifest's, because
    /// the declarations are what `schema::lower` leaks for the session and a
    /// rescan re-describes.
    #[test]
    fn a_declaration_past_the_headers_ceilings_is_refused_off_the_pipe() {
        let long = "a".repeat(lumit_lfx_abi::LFX_MAX_STRING_BYTES as usize + 1);
        let a_row = |kind: Declared| Declaration {
            id: "gain".into(),
            label: "Gain".into(),
            unit: lumit_core::fx::Unit::Raw,
            flags: 0,
            kind,
        };
        let a_slider = || Declared::Slider {
            default: 0.0,
            range: (0.0, 1.0),
            log: false,
        };
        let described = |params: Vec<Declaration>| DescribedPlugin {
            identity: identity("com.example.blur"),
            params,
            ..DescribedPlugin::default()
        };

        // A label past `LFX_MAX_STRING_BYTES` is the plainest case, and the one
        // the count of rows admits without a word.
        let mut row = a_row(a_slider());
        row.label.clone_from(&long);
        let refused = described(vec![row])
            .checked()
            .expect_err("a label nobody could draw");
        assert!(
            matches!(
                refused,
                LfxRejection::PastCeiling {
                    ceiling: Ceiling::StringBytes,
                    subject: "a control label",
                    ..
                }
            ),
            "{refused}"
        );

        // And every list inside a declaration has its own number.
        let cases: [(Declared, Ceiling); 5] = [
            (
                Declared::Choice {
                    options: vec!["one".to_owned(); lumit_lfx_abi::LFX_MAX_OPTIONS as usize + 1],
                    default: 0,
                    dividers_after: Vec::new(),
                },
                Ceiling::Options,
            ),
            (
                Declared::Choice {
                    options: vec!["one".to_owned()],
                    default: 0,
                    dividers_after: vec![0; lumit_lfx_abi::LFX_MAX_DIVIDERS as usize + 1],
                },
                Ceiling::Dividers,
            ),
            (
                Declared::File {
                    filter: vec!["cube".to_owned(); lumit_lfx_abi::LFX_MAX_FILTERS as usize + 1],
                    filter_name: "Lookup tables".into(),
                },
                Ceiling::Filters,
            ),
            (
                Declared::Curve {
                    default: vec![[0.0, 0.0]; lumit_lfx_abi::LFX_MAX_CURVE_POINTS as usize + 1],
                },
                Ceiling::CurvePoints,
            ),
            (
                Declared::File {
                    filter: Vec::new(),
                    filter_name: long.clone(),
                },
                Ceiling::StringBytes,
            ),
        ];
        for (kind, ceiling) in cases {
            let refused = described(vec![a_row(kind)])
                .checked()
                .expect_err("a list nobody could have declared");
            match refused {
                LfxRejection::PastCeiling { ceiling: got, .. } => assert_eq!(got, ceiling),
                other => panic!("expected a ceiling, got {other}"),
            }
        }

        // A heading's own two strings are asked the same question, and so are
        // the two lists that belong to the plugin rather than to a row.
        let mut with_a_heading = described(vec![a_row(a_slider())]);
        with_a_heading.groups.push(GroupRun {
            id: "shape".into(),
            label: long.clone(),
            hidden: false,
            first: 0,
            len: 1,
        });
        assert!(
            with_a_heading.checked().is_err(),
            "a heading nobody could draw crossed"
        );

        let mut talkative = described(vec![a_row(a_slider())]);
        talkative.report =
            vec![LfxRejection::NoCategoryDeclared; lumit_lfx_abi::LFX_MAX_PARAMS as usize + 1];
        assert!(
            talkative.checked().is_err(),
            "a report longer than the panel it is about crossed"
        );

        // And the honest record crosses whole.
        assert!(described(vec![a_row(a_slider())]).checked().is_ok());
    }

    /// `LFX_MAX_EFFECTS_PER_BUNDLE`, on both of the answers that carry a list
    /// of plugins. A manifest crafted to fill the transport's 8 MiB is of the
    /// order of a hundred thousand records, every one of which the Addons page
    /// would list and the roster would store.
    #[test]
    fn more_plugins_than_a_bundle_may_hold_are_refused() {
        let limit = lumit_lfx_abi::LFX_MAX_EFFECTS_PER_BUNDLE as usize;
        let entries = vec![identity("com.example.blur"); limit + 1];
        let refused = BrokerMessage::Manifested {
            entries: entries.clone(),
        }
        .checked()
        .expect_err("a manifest nobody could have written");
        assert!(
            matches!(
                refused,
                LfxRejection::PastCeiling {
                    ceiling: Ceiling::EffectsPerBundle,
                    subject: "the manifest",
                    given,
                } if given == limit as u64 + 1
            ),
            "{refused}"
        );

        let plugins: Vec<DescribedPlugin> = entries
            .iter()
            .map(|identity| DescribedPlugin {
                identity: identity.clone(),
                ..DescribedPlugin::default()
            })
            .collect();
        let refused = BrokerMessage::Described {
            plugins,
            refused: Vec::new(),
            report: Vec::new(),
        }
        .checked()
        .expect_err("a module nobody could have built");
        assert!(
            matches!(
                refused,
                LfxRejection::PastCeiling {
                    ceiling: Ceiling::EffectsPerBundle,
                    subject: "the module",
                    ..
                }
            ),
            "{refused}"
        );

        // A bundle of exactly the ceiling crosses, and every identity in it is
        // checked on the way.
        let full = vec![identity("com.example.blur"); limit];
        assert!(BrokerMessage::Manifested { entries: full }
            .checked()
            .is_ok());
        let mut one_bad = vec![identity("com.example.blur"); 3];
        if let Some(last) = one_bad.last_mut() {
            last.name = "a".repeat(lumit_lfx_abi::LFX_MAX_STRING_BYTES as usize + 1);
        }
        assert!(
            BrokerMessage::Manifested { entries: one_bad }
                .checked()
                .is_err(),
            "the count is not the only thing read"
        );
    }

    /// `LFX_MAX_LOG_BYTES`. A note is **cut, never refused** - a plugin that
    /// says too much should still be heard - and never in the middle of a
    /// character, because a `String` that is not UTF-8 is not a `String`. With
    /// §3.5's sixty-four retained notes, an uncut one is half a gigabyte of a
    /// stranger's text in the host process.
    #[test]
    fn a_note_longer_than_the_header_admits_is_cut_on_a_character_boundary() {
        let limit = lumit_lfx_abi::LFX_MAX_LOG_BYTES as usize;
        // Three bytes to the character, so the ceiling does not fall on a
        // boundary and the walk back has somewhere to go.
        let said = "€".repeat(limit);
        let cut = match (BrokerMessage::Note {
            kind: NoteKind::Warn,
            text: said.clone(),
        })
        .checked()
        .expect("a note is cut rather than refused")
        {
            BrokerMessage::Note { text, .. } => text,
            other => panic!("a note came back as {}", other.name()),
        };
        assert!(cut.len() <= limit, "{} bytes", cut.len());
        assert!(
            limit - cut.len() < 3,
            "cut at the last character boundary before the ceiling, not well short of it"
        );
        assert!(
            said.starts_with(&cut),
            "and it is the front of what was said"
        );

        // The badge sentence is a log line by another name.
        let cut = match (BrokerMessage::Failed {
            action: HostAction::Process,
            message: said,
        })
        .checked()
        .expect("a failure is cut rather than refused")
        {
            BrokerMessage::Failed { message, .. } => message,
            other => panic!("a failure came back as {}", other.name()),
        };
        assert!(cut.len() <= limit);

        // A note inside the ceiling is untouched.
        let short = match (BrokerMessage::Note {
            kind: NoteKind::Info,
            text: "a sentence".into(),
        })
        .checked()
        .expect("nothing to cut")
        {
            BrokerMessage::Note { text, .. } => text,
            other => panic!("a note came back as {}", other.name()),
        };
        assert_eq!(short, "a sentence");
    }

    /// `LFX_MAX_TEMPORAL_WINDOW`, on the offsets a plugin says it still needs.
    /// §4.2's table calls them clamped ±64 and they reach the frame key and the
    /// neighbour decode, so they are held to the window here exactly as
    /// [`DeclaredTraits::temporal_window`] holds the declaration - and the same
    /// neighbour asked for twice is one frame, which is what bounds the list.
    #[test]
    fn the_frames_a_plugin_asks_for_are_held_to_the_window_the_header_admits() {
        let limit = lumit_lfx_abi::LFX_MAX_TEMPORAL_WINDOW;
        let held = |frames: Vec<i32>| -> Vec<i32> {
            match (BrokerMessage::Processed {
                slot: 0,
                frames_needed: frames,
            })
            .checked()
            .expect("frames are held, never refused")
            {
                BrokerMessage::Processed { frames_needed, .. } => frames_needed,
                other => panic!("a render answer came back as {}", other.name()),
            }
        };

        assert_eq!(
            held(vec![-10_000, 1, 0, -1, 1, 10_000]),
            vec![-limit, -1, 0, 1, limit],
            "clamped, ordered, and each frame asked for once"
        );
        assert_eq!(
            held(Vec::new()),
            Vec::<i32>::new(),
            "a plugin that wants nothing"
        );

        let spam = held((0..10_000).collect());
        assert!(
            spam.len() <= 2 * limit as usize + 1,
            "the window bounds the list however long it arrived: {}",
            spam.len()
        );
    }

    /// The gate is a gate, not a rewrite: a message from a broker that is
    /// inside every one of the header's ceilings crosses it unchanged.
    #[test]
    fn a_message_inside_every_ceiling_crosses_the_gate_unchanged() {
        for message in every_broker_message() {
            let before = format!("{message:?}");
            let after = message.checked().expect("nothing here is past a ceiling");
            assert_eq!(format!("{after:?}"), before, "the gate changed {before}");
        }
    }
}
