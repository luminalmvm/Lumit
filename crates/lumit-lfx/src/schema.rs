//! A described plugin, turned into the declaration a built-in effect carries
//! (docs/impl/lfx.md §2.3, §2.4).
//!
//! # In plain terms
//!
//! Lumit's own effects are declared once, in a struct that says what the effect
//! is called, what family it belongs to, how expensive it is, and what its
//! controls are (docs/impl/effect-registry.md). Everything downstream - the
//! Add-effect menu, the Effect Controls panel, keyframes, expressions, the
//! cache key - reads that one declaration. A plugin has just told us the same
//! facts in LFX's words. This module writes them down in Lumit's, so an LFX
//! effect and a built-in are the same kind of thing to everything that comes
//! after (docs/12 §1).
//!
//! # A lowering, and not a mirror
//!
//! `lfx_param_kind` **lowers onto** [`ParamKind`] rather than copying it. There
//! is nothing for a frozen C enum to mirror: `ParamKind` is a data-carrying
//! Rust enum with no `#[repr]` and so no stable discriminants, its order is
//! arrival order, and the correspondence is one-to-one in neither direction.
//! [`ParamKind::ColourName`], [`ParamKind::Layer`], [`ParamKind::Clip`] and
//! [`ParamKind::MaskPath`] have no LFX counterpart at all, and `POINT2`,
//! `POINT3` and `GROUP` are not `ParamKind` variants. So the table is written
//! here, honestly, as a lowering:
//!
//! * **`FLOAT`, `SLIDER`, `INT`, `BOOL`, `CHOICE`, `COLOUR`, `ANGLE`, `SEED`,
//!   `CURVE`, `FILE` and `ACTION`** each become the `ParamKind` of the same
//!   name. LFX declares a slider's logarithm and a dropdown's dividers, which
//!   OFX has no way of saying.
//! * **`POINT2` becomes two rows** and `POINT3` three, `<id>_x` / `_y` / `_z`.
//!   Lumit has deliberately no Point kind: a point is two adjacent number rows
//!   the panel folds into one crosshair, which is why
//!   [`EffectSchema::pairs`] reads the suffixes. A plugin's Centre therefore
//!   draws exactly as a built-in's does, link glyph and all.
//! * **`GROUP` is a run, not a row.** It becomes a [`ParamGroup`] over the rows
//!   declared inside it.
//! * **`PATH` and `STRING` are refused by name at the tag.** The frozen sink
//!   has an entry point per kind it admits and neither of those two has one, so
//!   a version 1 plugin cannot declare either through the ABI at all; the
//!   refusal is made where a declaration arrives by another road - the broker,
//!   decoding a proto message ([`crate::describe::Describe::decline_kind`]) -
//!   and is a line in the scan report rather than a row here.
//!
//! The seam nothing else watches is the *other* direction: the layout suite
//! binds the header to `lumit-lfx-abi`'s Rust mirror, and cannot bind either to
//! `ParamKind`, because `ParamKind` is not what the header is a copy of. So the
//! lowering carries [`carriage`] - an exhaustive `match` over `ParamKind` with
//! no `_` arm. The next variant added to it fails this build and forces a
//! deliberate answer, admit it or refuse it by name, instead of becoming a row
//! whose value silently reaches nothing.
//!
//! # What the unit is, and what it is not
//!
//! Unit is mandatory, and unstated is a describe refusal - the same build
//! failure every built-in faces in
//! `every_parameter_declares_a_deliberate_unit`, given to a stranger's effect
//! at describe time. `LFX_UNIT_PX` means **pixels at composition size**, never
//! pixels of the buffer handed over, which is the most useful thing LFX takes
//! from being first-party: OFX's own unit reading must answer [`Unit::Raw`] for
//! every normalised spatial type, because the standard does not say.
//!
//! A control that cannot carry a unit at all - a switch, a dropdown, a colour,
//! a seed, a curve, a file, a button - is [`Unit::Raw`] whatever it declared,
//! and an angle is [`Unit::Degrees`] by definition. That is the default
//! `#[derive(Effect)]` reaches for when a built-in's author says nothing;
//! **a built-in's author may then say otherwise and a plugin's may not**, since
//! the frozen header's own words are that an angle's unit *must* be
//! `LFX_UNIT_DEGREES`. So it is applied here as a normalisation with a line in
//! the report ([`LfxRejection::UnitIgnoredForKind`]) rather than as a silent
//! rewrite: an author whose declaration was ignored is owed the sentence saying
//! so, which is the whole of what the vendor's own validator run is for.
//!
//! # The strings are leaked, once
//!
//! [`EffectSchema`] is a `'static` declaration because a built-in's is a
//! compile-time constant; a plugin's is discovered at start-up and then lives
//! as long as the session, so leaking it is the honest spelling of that
//! lifetime rather than a leak in the sense that matters.
//!
//! Which is why the descriptor's own three strings are asked
//! `LFX_MAX_STRING_BYTES` **before** anything is minted from them, as the sink
//! asks it of every row's: the id, the name and the vendor arrive here rather
//! than through a sink, and what is leaked here is leaked for the session.
//! [`LfxRejection::IdentityStringTooLong`] is structural where a row's is a
//! line - a row this build cannot draw costs that one control, and a plugin
//! whose id cannot be carried has no match name for a project file to store.
//!
//! Leaked **once**, and the two readers that want a row's id back take the
//! built schema beside the descriptor rather than minting it again:
//! [`value_routes`] is the map discovery turns a resolved bag into the dense value
//! array with, which is once per frame per instance, and a second minting there
//! would leak one fresh copy of every row of every LFX instance per rendered
//! frame, for ever and with no ceiling. [`hidden_rows`] answers in the
//! `&'static str`s the registry hook already wants for the same reason.
//!
//! # Thread role
//!
//! Control thread. Nothing here touches a plugin; it reads what one already
//! said.

use std::collections::{BTreeSet, HashMap};

use lumit_core::fx::{
    CostClass, EffectSchema, EffectTraits, FxCategory, MatteRole, ParamGroup, ParamId, ParamKind,
    ParamSchema, Roi, Unit, LFX_MATCH_PREFIX,
};
use lumit_lfx_abi::{
    LfxCategory, LfxParamKind, LfxUnit, LFX_ALPHA_STRAIGHT, LFX_CATEGORY_BLUR_SHARPEN,
    LFX_CATEGORY_COLOUR, LFX_CATEGORY_DISTORTION, LFX_CATEGORY_GENERATE, LFX_CATEGORY_STYLISE,
    LFX_CATEGORY_TEMPORAL, LFX_CATEGORY_TRANSITION, LFX_CATEGORY_UNSET, LFX_CATEGORY_UTILITY,
    LFX_COST_CHEAP, LFX_COST_MODERATE, LFX_COST_TRIVIAL, LFX_MAX_CATEGORIES,
    LFX_MAX_REQUIRED_EXTENSIONS, LFX_MAX_TEMPORAL_WINDOW, LFX_PARAM_ACTION, LFX_PARAM_ANGLE,
    LFX_PARAM_BOOL, LFX_PARAM_CHOICE, LFX_PARAM_COLOUR, LFX_PARAM_CURVE, LFX_PARAM_FILE,
    LFX_PARAM_SEED, LFX_ROI_EXACT, LFX_ROI_FULL_FRAME, LFX_ROI_PADDED, LFX_TRAIT_SEEDED,
    LFX_UNIT_DEGREES, LFX_UNIT_FRAMES, LFX_UNIT_PCT_DIAG, LFX_UNIT_PERCENT, LFX_UNIT_PX,
    LFX_UNIT_RAW, LFX_UNIT_SECONDS,
};

use crate::describe::{over_long, Declaration, Declared, PluginDescriptor, Traits};
use crate::{version, LfxRejection};

/// Turn a described plugin into the declaration Lumit's own effects carry.
///
/// # Errors
///
/// [`LfxRejection::VersionOutOfRange`] for a release the frame key could not
/// tell apart from another one, [`LfxRejection::DuplicateParamId`] if two rows
/// would land on the same [`ParamId`], and
/// [`LfxRejection::TemporalWindowUnusable`] for a window of neighbouring frames
/// nobody can honour. The unit refusals are met earlier, in the sink, because
/// a unit is refused on the call that declared it.
pub fn schema_of(plugin: &PluginDescriptor) -> Result<EffectSchema, LfxRejection> {
    let identity = &plugin.identity;
    // The descriptor's own three strings, asked the ceiling the sink already
    // asks of every row's, and asked **before** anything is minted from them:
    // `match_name` is what a project file stores, so an id nobody can carry is
    // a name nobody can write down.
    for (field, text) in [
        ("id", &identity.id),
        ("name", &identity.name),
        ("vendor", &identity.vendor),
    ] {
        if let Some(bytes) = over_long(text) {
            return Err(LfxRejection::IdentityStringTooLong {
                field: field.to_owned(),
                bytes,
            });
        }
    }
    let version = version::mint(identity.major, identity.minor, identity.patch)?;
    let traits = traits_of(identity.traits.as_ref())?;

    let mut rows: Vec<ParamSchema> = Vec::new();
    // Where each declaration's rows start, so a heading's run of declarations
    // becomes a run of rows without anybody counting twice.
    let mut starts: Vec<(usize, usize)> = Vec::new();
    for declaration in &plugin.params {
        let minted = rows_of(declaration);
        starts.push((rows.len(), minted.len()));
        rows.extend(minted);
    }

    // Two rows under one id is two controls the panel cannot tell apart and one
    // value in the bag. The sink catches this as it happens, so that a plugin
    // gets its `false` on the call that made the duplicate; it is asked again
    // here because a descriptor need not have come through a sink at all.
    // And the host's own corner of that bag is not a stranger's to declare, for
    // the same reason and asked in the same two places: `resolve_derived`
    // overwrites whatever is on a `derived.` row, so a plugin that reached the
    // catalogue with one would draw a control that does nothing.
    let mut minted: HashMap<ParamId, &'static str> = HashMap::with_capacity(rows.len());
    for row in &rows {
        if row.id.starts_with(crate::def::DERIVED_PREFIX) {
            return Err(LfxRejection::ReservedParamId {
                id: row.id.to_owned(),
            });
        }
        if let Some(first) = minted.insert(ParamId::new(row.id), row.id) {
            return Err(LfxRejection::DuplicateParamId {
                first: first.to_owned(),
                second: row.id.to_owned(),
            });
        }
    }

    let mut groups: Vec<ParamGroup> = Vec::new();
    for run in &plugin.groups {
        let members: Vec<&'static str> = run_rows(&rows, &starts, run.first, run.len)
            .map(|row| row.id)
            .collect();
        if members.is_empty() {
            continue;
        }
        groups.push(ParamGroup {
            label: leak(&run.label),
            params: leak_slice(members),
            // LFX declares no open state: a heading that starts closed is an
            // author's guess at what a person is not interested in, and the
            // panel remembers the twirl itself.
            collapsed: false,
            visible_when: None,
            visible_when_lens_elements: None,
        });
    }

    Ok(EffectSchema {
        match_name: leak(&format!("{LFX_MATCH_PREFIX}{}", identity.id)),
        label: leak(&identity.name),
        version,
        category: families(plugin)
            .first()
            .copied()
            .unwrap_or(FxCategory::Utility),
        traits,
        params: leak_slice(rows),
        groups: leak_slice(groups),
        // Greying rules are a built-in's declaration about its own controls.
        // LFX version 1 gives an author no way to say one, and a rule the host
        // invented would be a claim about somebody else's panel.
        enabled_when: &[],
        // No matte row (docs/impl/lfx.md §2.4), as OFX and the audio hosts
        // declare: injecting one would put a control on the panel the plugin
        // has never heard of and nothing would consume.
        matte: MatteRole::None,
    })
}

/// Every line this plugin puts in the scan report: what the sink declined,
/// followed by what the lowering itself could not take at face value.
///
/// It exists so that "this plugin has a control Lumit cannot show" is a line in
/// the report rather than a silence - the shape the OFX host's
/// `unrepresented()` already has at the same seam. Every entry answers `false`
/// to [`LfxRejection::refuses_the_effect`]: the ones that do not are returned
/// by [`crate::describe::Describe::finish`] and [`schema_of`] instead.
#[must_use]
pub fn notes(plugin: &PluginDescriptor) -> Vec<LfxRejection> {
    let identity = &plugin.identity;
    let mut notes = plugin.report.clone();
    notes.extend(family_notes(&identity.categories));
    let required = count_of(identity.required_extensions.len());
    if required > LFX_MAX_REQUIRED_EXTENSIONS {
        notes.push(LfxRejection::TooManyRequiredExtensions { declared: required });
    }
    if let Some(traits) = &identity.traits {
        if traits.roi_kind == LFX_ROI_PADDED && padding_of(traits).is_none() {
            notes.push(LfxRejection::PaddingWithoutDistance);
        }
    }
    notes
}

/// The extensions this effect cannot work without, read up to the ABI's own
/// ceiling.
///
/// **One predicate, read from both ends**, as [`padding_of`] already is for the
/// region of interest: [`notes`] files
/// [`LfxRejection::TooManyRequiredExtensions`] for exactly the overflow this
/// drops, so the sentence on the page and the list the negotiation answers
/// cannot disagree. Every reader of the list goes through here - the
/// declaration itself is what a stranger wrote, and nothing downstream should
/// have to remember to bound it again.
#[must_use]
pub fn required_extensions(plugin: &PluginDescriptor) -> &[String] {
    let declared = &plugin.identity.required_extensions;
    declared
        .get(..declared.len().min(LFX_MAX_REQUIRED_EXTENSIONS as usize))
        .unwrap_or(declared)
}

/// The picture families this effect claims, the **heading first**.
///
/// The first declared category is the schema's category and the Add-effect
/// heading; the rest are search keywords the discovery record carries. This is
/// where LFX earns docs/12 §3.7 and OFX cannot: an OFX grouping is somebody
/// else's taxonomy, so the bridge gives it a heading of its own, while an LFX
/// author declares against Lumit's own vocabulary - which is something they can
/// do and an OFX author cannot.
///
/// A number outside the closed vocabulary lands in [`FxCategory::Utility`] plus
/// a report line, so nothing else changes. `LFX_CATEGORY_UNSET` is not one of
/// those: it is the vendor having filled nothing in rather than a number from a
/// newer header, so it claims no family and adds no keyword, and a list of
/// nothing else is [`LfxRejection::NoCategoryDeclared`]. Repeats are dropped,
/// keeping the first mention: a keyword list that said Colour twice would put
/// the same search term in twice.
#[must_use]
pub fn families(plugin: &PluginDescriptor) -> Vec<FxCategory> {
    let mut families: Vec<FxCategory> = Vec::new();
    // At most the ceiling the ABI declares, which is one per member of the
    // closed vocabulary: a list longer than that is a list with repeats in it,
    // and `family_notes` says so.
    for declared in plugin
        .identity
        .categories
        .iter()
        .take(LFX_MAX_CATEGORIES as usize)
        .filter(|declared| **declared != LFX_CATEGORY_UNSET)
    {
        let family = family_of(*declared).unwrap_or(FxCategory::Utility);
        if !families.contains(&family) {
            families.push(family);
        }
    }
    if families.is_empty() {
        families.push(FxCategory::Utility);
    }
    families
}

/// What the host schedules this effect from.
///
/// **Every trait field's zero means "unstated", and the lowering is where that
/// becomes the pessimistic answer rather than discriminant nought.** A `None`
/// block, a `memset` one and a short one whose tail this header has and the
/// plugin's did not are the same declaration, and all three schedule as
/// [`CostClass::Heavy`] with a [`Roi::FullFrame`] region. docs/13:297 is
/// explicit about which way the failure runs: claiming less reach than the
/// kernel uses produces tile seams, and a seam is a correctness bug where a
/// wasted read is only slow.
///
/// # Errors
///
/// [`LfxRejection::TemporalWindowUnusable`] for a declared window that does not
/// contain the frame being rendered, or reaches past
/// [`LFX_MAX_TEMPORAL_WINDOW`] either way. A refusal rather than a clamp: a
/// clamp would hand the effect a different window from the one it says it
/// reads, which is the seam again by another road.
pub fn traits_of(declared: Option<&Traits>) -> Result<EffectTraits, LfxRejection> {
    let Some(traits) = declared else {
        return Ok(EffectTraits {
            cost: CostClass::Heavy,
            roi: Roi::FullFrame,
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        });
    };

    let (lo, hi) = (traits.temporal_lo, traits.temporal_hi);
    if lo > 0 || hi < 0 || lo < -LFX_MAX_TEMPORAL_WINDOW || hi > LFX_MAX_TEMPORAL_WINDOW {
        return Err(LfxRejection::TemporalWindowUnusable { lo, hi });
    }

    Ok(EffectTraits {
        // Unstated, and anything outside the enumeration, is heavy: a plugin is
        // somebody else's code across a process boundary, and the degradation
        // ordering should give it up first.
        cost: match traits.cost {
            LFX_COST_TRIVIAL => CostClass::Trivial,
            LFX_COST_CHEAP => CostClass::Cheap,
            LFX_COST_MODERATE => CostClass::Moderate,
            _ => CostClass::Heavy,
        },
        roi: match traits.roi_kind {
            LFX_ROI_EXACT => Roi::Exact,
            // A padding with no distance is a padding that pads by nothing, and
            // the honest reading of it is exactly - with a line in the report,
            // since the plugin plainly meant to say something. The distance is
            // asked through `padding_of`, which `notes` asks too, so the schema
            // and the report cannot disagree about which declarations pad.
            LFX_ROI_PADDED => padding_of(traits).map_or(Roi::Exact, Roi::PaddedPx),
            LFX_ROI_FULL_FRAME => Roi::FullFrame,
            _ => Roi::FullFrame,
        },
        temporal: temporal_window(lo, hi),
        // Two states, not three: `premultiplied` is a `bool`, so an "ignores"
        // and a "straight" would both land on `false` and the host would
        // unpremultiply before an effect that did not want touching.
        premultiplied: traits.alpha != LFX_ALPHA_STRAIGHT,
        seeded: traits.flags & LFX_TRAIT_SEEDED != 0,
        // LFX is a picture ABI: `lfx.audio` is a reserved extension id and a
        // version 1 header, and nothing a v1 plugin can ask for.
        beat_input: false,
    })
}

/// One schema row's way **back** to the declaration it came from.
///
/// The trip out is [`schema_of`]: a plugin's declarations become Lumit rows,
/// and a point becomes two or three of them. The trip home is this: a resolved
/// bag, keyed by [`ParamId`], has to become the dense array `process` reads its
/// values out of, and the bag has only the hashes - the names are gone. So the
/// routes are worked out once, from the same minting that made the rows, and
/// nothing has to guess at the reverse of a suffix rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValueRoute {
    /// The row, as the bag keys it.
    pub id: ParamId,
    /// The same row by its schema id, which is how the document names it.
    pub row: &'static str,
    /// Which declaration this row is part of, as an index into
    /// [`PluginDescriptor::params`].
    pub declaration: usize,
    /// Which element of the dense value array the declaration is - the number
    /// `lfx_value.param` carries, counting only the declarations that carry a
    /// value and counting a point once.
    pub element: u32,
    /// The declaration's own kind, which names the arm of the union its value
    /// is read through.
    pub kind: LfxParamKind,
    /// Which component of the declaration this row is; nought for the scalar
    /// kinds, which is most of them.
    pub component: usize,
    /// How many components the declaration has in all.
    pub dimension: usize,
}

/// Every schema row's route home, in schema order.
///
/// Declarations with no value - an Action, and a heading, which is not a
/// declaration at all - appear nowhere here, for the same reason they appear in
/// no bag: there is no value of theirs for Lumit to carry.
///
/// `schema` is the one [`schema_of`] built from this same descriptor, and the
/// rows are read **off** it rather than minted again: this is the map a
/// resolved bag becomes the dense value array through, which happens once per
/// frame per instance, and a fresh minting there would leak one copy of every
/// row of every LFX instance per rendered frame.
#[must_use]
pub fn value_routes(plugin: &PluginDescriptor, schema: &EffectSchema) -> Vec<ValueRoute> {
    let mut routes = Vec::new();
    let mut element: u32 = 0;
    for (declaration, declared, rows) in declared_rows(plugin, schema) {
        if !crosses(rows) {
            continue;
        }
        let dimension = rows.len();
        for (component, row) in rows.iter().enumerate() {
            routes.push(ValueRoute {
                id: ParamId::new(row.id),
                row: row.id,
                declaration,
                element,
                kind: declared.kind.tag(),
                component,
                dimension,
            });
        }
        element = element.saturating_add(1);
    }
    routes
}

/// The declarations that put an element in the dense value array, in the order
/// the elements are numbered in.
///
/// [`value_routes`] answers which element a *row* belongs to; this answers what
/// the elements themselves are. The two are one walk and one predicate -
/// [`crosses`] over [`carriage`] - so nothing can come to two opinions about
/// which declarations carry a value, and an opinion is exactly what would shift
/// every element after the declaration they disagreed about: the plugin would
/// then read correct-looking kind tags over the neighbouring row's value, which
/// is the fault §2.1's `value_stride` rule exists to make impossible seen from
/// the host's own side.
///
/// `LfxDef` reads it to fill the array with the plugin's own declared defaults
/// before a resolved bag is written over them.
#[must_use]
pub fn value_elements<'a>(
    plugin: &'a PluginDescriptor,
    schema: &EffectSchema,
) -> Vec<&'a Declaration> {
    declared_rows(plugin, schema)
        .filter(|(_, _, rows)| crosses(rows))
        .map(|(_, declaration, _)| declaration)
        .collect()
}

/// Whether the run of rows one declaration minted carries a value.
///
/// The one answer to that question, asked by [`value_routes`] and by
/// [`value_elements`] alike. A declaration that minted no row at all - which
/// nothing does today - carries nothing, for want of a kind to ask about.
fn crosses(rows: &[ParamSchema]) -> bool {
    rows.first()
        .is_some_and(|first| carriage(&first.kind).crosses())
}

/// Each declaration beside the run of schema rows it minted, in order.
///
/// The width of a run is the declaration's own axis list and nothing else
/// ([`Declared::axes`]), which is the same list [`row_ids`] spells the suffixes
/// from - so the pairing is read from the one place the spread is written down
/// rather than guessed at from the ids.
fn declared_rows<'a>(
    plugin: &'a PluginDescriptor,
    schema: &EffectSchema,
) -> impl Iterator<Item = (usize, &'a Declaration, &'static [ParamSchema])> {
    let params: &'static [ParamSchema] = schema.params;
    let mut cursor = 0_usize;
    plugin
        .params
        .iter()
        .enumerate()
        .map(move |(index, declaration)| {
            let width = declaration.kind.axes().map_or(1, <[&str]>::len);
            let end = cursor.saturating_add(width);
            let rows = params.get(cursor..end).unwrap_or_default();
            cursor = end;
            (index, declaration, rows)
        })
}

/// Every row the plugin marks hidden at describe time: the rows the panel
/// starts without.
///
/// The rows stay in the schema either way - a row that starts hidden has to be
/// there to appear later - and a hidden heading hides everything down to its
/// last row, which is how one flag hides a whole run.
///
/// Answered in the schema's own `&'static str`s, because the registry hook this
/// feeds is `fn hidden_rows(&self, _) -> Vec<&'static str>`: a fresh `String`
/// per row would be a third copy of every id, minted on every panel rebuild.
#[must_use]
pub fn hidden_rows(plugin: &PluginDescriptor, schema: &EffectSchema) -> BTreeSet<&'static str> {
    let runs: Vec<std::ops::Range<usize>> = plugin
        .groups
        .iter()
        .filter(|run| run.hidden)
        .map(|run| run.first..run.first.saturating_add(run.len))
        .collect();
    declared_rows(plugin, schema)
        .filter(|(index, declaration, _)| {
            declaration.hidden() || runs.iter().any(|run| run.contains(index))
        })
        .flat_map(|(_, _, rows)| rows.iter().map(|row| row.id))
        .collect()
}

/// What a declared `lfx_unit` means to Lumit.
///
/// `LFX_UNIT_UNSET` - and every number outside the enumeration, which is the
/// same statement made by a plugin built against a header this one has never
/// seen - lowers to [`Unit::Unset`], which is not a unit but the absence of a
/// decision. The sink refuses it; nothing downstream has to carry the raw
/// number or decide again what it meant.
#[must_use]
pub const fn unit_of(unit: LfxUnit) -> Unit {
    match unit {
        LFX_UNIT_RAW => Unit::Raw,
        LFX_UNIT_PERCENT => Unit::Percent,
        LFX_UNIT_PCT_DIAG => Unit::PctDiag,
        LFX_UNIT_PX => Unit::Px,
        LFX_UNIT_DEGREES => Unit::Degrees,
        LFX_UNIT_SECONDS => Unit::Seconds,
        LFX_UNIT_FRAMES => Unit::Frames,
        _ => Unit::Unset,
    }
}

/// Which arm of `lfx_value`'s union a row of this kind crosses in - and
/// therefore whether it crosses at all.
///
/// **This is the seam the layout suite cannot watch.** Those tests bind the
/// header to `lumit-lfx-abi`'s Rust mirror; neither is a copy of [`ParamKind`],
/// so neither can notice a variant added to it. The `match` below has no `_`
/// arm, so the next one fails this build and forces a deliberate answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Carriage {
    /// `v.f`, and a point's axes fold into `v.xy` or `v.xyz`.
    Double,
    /// `v.i`.
    Whole,
    /// `v.b`.
    Switch,
    /// `v.choice`.
    Chosen,
    /// `v.rgba`.
    Rgba,
    /// `v.curve`.
    Points,
    /// `v.file`, whose path the host fills from the auxiliary slot beside the
    /// op because only the host knows which file actually opened.
    Path,
    /// Nothing crosses, and nothing could: an Action asks the host to *do*
    /// something and has no value at all.
    NoValue,
    /// LFX mints no row of this kind, so the question does not arise. A layer
    /// reference, a clip, a mask path and an OCIO name are all references into
    /// the document, and an out-of-process plugin holds no document.
    Unminted,
}

impl Carriage {
    /// Whether a row of this kind puts an element in the dense array.
    #[must_use]
    pub const fn crosses(self) -> bool {
        !matches!(self, Carriage::NoValue | Carriage::Unminted)
    }
}

/// The arm a row's value crosses in. See [`Carriage`] for why this has no `_`.
#[must_use]
pub const fn carriage(kind: &ParamKind) -> Carriage {
    match kind {
        ParamKind::Float { .. } | ParamKind::Slider { .. } | ParamKind::Angle { .. } => {
            Carriage::Double
        }
        ParamKind::Int { .. } | ParamKind::Seed => Carriage::Whole,
        ParamKind::Bool { .. } => Carriage::Switch,
        ParamKind::Choice { .. } => Carriage::Chosen,
        ParamKind::Colour { .. } => Carriage::Rgba,
        ParamKind::Curve { .. } => Carriage::Points,
        ParamKind::File { .. } => Carriage::Path,
        ParamKind::Action => Carriage::NoValue,
        ParamKind::ColourName { .. }
        | ParamKind::Layer { .. }
        | ParamKind::Clip
        | ParamKind::MaskPath { .. } => Carriage::Unminted,
    }
}

/// The ids of the rows one declaration mints: its own for a scalar, `<id>_x`
/// and the rest for a point.
///
/// **The suffix rule is spelled here and nowhere else.** [`rows_of`] leaks
/// these, [`value_routes`] reads them back and the sink checks them for
/// collisions, so nothing reverses the rule by guesswork.
pub(crate) fn row_ids(declaration: &Declaration) -> Vec<String> {
    match declaration.kind.axes() {
        None => vec![declaration.id.clone()],
        Some(axes) => axes
            .iter()
            .map(|axis| format!("{}_{axis}", declaration.id))
            .collect(),
    }
}

/// The schema rows one declaration becomes: one for a scalar, two or three for
/// a point.
fn rows_of(declaration: &Declaration) -> Vec<ParamSchema> {
    let unit = row_unit(declaration.unit, declaration.kind.tag());
    let ids = row_ids(declaration);
    let spread = |axis_labels: &[&str], kind: &dyn Fn(usize) -> ParamKind| {
        ids.iter()
            .enumerate()
            .map(|(index, id)| ParamSchema {
                id: leak(id),
                label: leak(&format!(
                    "{} {}",
                    declaration.label,
                    axis_labels.get(index).copied().unwrap_or_default()
                )),
                kind: kind(index),
                unit,
            })
            .collect()
    };
    let one = |kind: ParamKind| {
        vec![ParamSchema {
            id: leak(&declaration.id),
            label: leak(&declaration.label),
            kind,
            unit,
        }]
    };

    match &declaration.kind {
        Declared::Float {
            default,
            slider,
            hard,
        } => one(ParamKind::Float {
            default: *default,
            slider: *slider,
            hard: *hard,
        }),
        Declared::Slider {
            default,
            range,
            log,
        } => one(ParamKind::Slider {
            default: *default,
            range: *range,
            log: *log,
        }),
        Declared::Int {
            default,
            slider,
            hard,
        } => one(ParamKind::Int {
            default: *default,
            slider: *slider,
            hard: *hard,
        }),
        Declared::Angle { default, dial_step } => one(ParamKind::Angle {
            default: *default,
            dial_step: *dial_step,
        }),
        Declared::Bool { default } => one(ParamKind::Bool { default: *default }),
        Declared::Choice {
            options,
            default,
            dividers_after,
        } => {
            let options: Vec<&'static str> = options.iter().map(|option| leak(option)).collect();
            one(ParamKind::Choice {
                options: leak_slice(options),
                default: *default,
                dividers_after: leak_slice(dividers_after.clone()),
            })
        }
        Declared::Colour { default, range } => one(ParamKind::Colour {
            default: *default,
            range: *range,
        }),
        Declared::Seed => one(ParamKind::Seed),
        Declared::Point2 { default, slider } => {
            let axes = [default.0, default.1];
            spread(&["X", "Y"], &|index| ParamKind::Float {
                default: axes.get(index).copied().unwrap_or_default(),
                slider: *slider,
                hard: (None, None),
            })
        }
        Declared::Point3 { default, slider } => {
            let axes = [default.0, default.1, default.2];
            spread(&["X", "Y", "Z"], &|index| ParamKind::Float {
                default: axes.get(index).copied().unwrap_or_default(),
                slider: *slider,
                hard: (None, None),
            })
        }
        Declared::Curve { default } => one(ParamKind::Curve {
            default: leak_slice(default.clone()),
        }),
        Declared::File {
            filter,
            filter_name,
        } => {
            let filter: Vec<&'static str> = filter.iter().map(|ext| leak(ext)).collect();
            one(ParamKind::File {
                filter: leak_slice(filter),
                filter_name: leak(filter_name),
            })
        }
        Declared::Action => one(ParamKind::Action),
    }
}

/// The unit a kind is in **whatever** the plugin declared, or `None` for the
/// kinds whose unit is the plugin's own to choose.
///
/// A switch, a dropdown, a colour, a seed, a tone curve, a file and a button
/// are in no unit, and an angle is in degrees by definition. The frozen header
/// says as much - an angle's unit *must* be `LFX_UNIT_DEGREES` - so a
/// declaration that says otherwise is normalised here **and** named in the scan
/// report: [`crate::describe::Describe::declare`] asks this same question and
/// pushes [`LfxRejection::UnitIgnoredForKind`] when the answer differs from what
/// the plugin wrote. One predicate, so the row and the sentence beside it
/// cannot disagree.
#[must_use]
pub const fn forced_unit(kind: LfxParamKind) -> Option<Unit> {
    match kind {
        LFX_PARAM_ANGLE => Some(Unit::Degrees),
        LFX_PARAM_BOOL | LFX_PARAM_CHOICE | LFX_PARAM_COLOUR | LFX_PARAM_SEED | LFX_PARAM_CURVE
        | LFX_PARAM_FILE | LFX_PARAM_ACTION => Some(Unit::Raw),
        _ => None,
    }
}

/// Whether a row of this kind is one a person may keyframe here.
///
/// The frozen header's `LFX_PARAM_FLAG_STATIC` says a row "never keyframes: one
/// value for the whole of the effect's life, as a file choice or a curve is" -
/// and a tone curve, a file choice and a button are exactly that in Lumit
/// already, so the flag asks for nothing on them. Every other kind animates,
/// and [`ParamSchema`] has no field to say otherwise, so the flag is read,
/// dropped and named:
/// [`crate::describe::Describe::declare`] pushes
/// [`LfxRejection::StaticRowAnimatesAnyway`] for exactly the kinds this answers
/// `true` for. One predicate, so the row and the sentence beside it cannot
/// disagree.
#[must_use]
pub const fn animates(kind: LfxParamKind) -> bool {
    !matches!(kind, LFX_PARAM_CURVE | LFX_PARAM_FILE | LFX_PARAM_ACTION)
}

/// What a row of this kind is actually *in*, which is not always what the
/// plugin declared.
fn row_unit(declared: Unit, kind: LfxParamKind) -> Unit {
    match forced_unit(kind) {
        Some(fixed) => fixed,
        None => declared,
    }
}

/// The rows a run of `len` declarations starting at `first` minted.
fn run_rows<'a>(
    rows: &'a [ParamSchema],
    starts: &[(usize, usize)],
    first: usize,
    len: usize,
) -> impl Iterator<Item = &'a ParamSchema> {
    let run = starts.get(first..first.saturating_add(len)).unwrap_or(&[]);
    let start = run.first().map_or(0, |(start, _)| *start);
    let count: usize = run.iter().map(|(_, count)| *count).sum();
    rows.get(start..start.saturating_add(count))
        .unwrap_or_default()
        .iter()
}

/// The picture family a declared category is, or `None` for one outside the
/// closed vocabulary of eight.
///
/// `Audio`, `Drivers` and `Controls` are deliberately not in the ABI's
/// enumeration: a picture plugin declaring "this is a driver" would be claiming
/// a data signature it has not got. Nor is `Compositing`, which the Add-effect
/// menu filters out by name and routes to the node graph's console instead - a
/// legal value that would leave a plugin registered, badged nowhere and in no
/// menu a layer can reach.
const fn family_of(declared: LfxCategory) -> Option<FxCategory> {
    match declared {
        LFX_CATEGORY_BLUR_SHARPEN => Some(FxCategory::BlurSharpen),
        LFX_CATEGORY_COLOUR => Some(FxCategory::Colour),
        LFX_CATEGORY_DISTORTION => Some(FxCategory::Distortion),
        LFX_CATEGORY_GENERATE => Some(FxCategory::Generate),
        LFX_CATEGORY_STYLISE => Some(FxCategory::Stylise),
        LFX_CATEGORY_TEMPORAL => Some(FxCategory::Temporal),
        LFX_CATEGORY_TRANSITION => Some(FxCategory::Transition),
        LFX_CATEGORY_UTILITY => Some(FxCategory::Utility),
        _ => None,
    }
}

/// The report lines a category list puts in: one per *distinct* number outside
/// the vocabulary, one for a list past the ABI's own ceiling, and one for a
/// descriptor that named no family at all.
///
/// Deduped the same way [`families`] dedupes the families themselves, and for
/// the same reason: a list declaring 99 three times is one mistake, and three
/// identical sentences on the Addons page say nothing the first did not.
fn family_notes(declared: &[LfxCategory]) -> Vec<LfxRejection> {
    // `LFX_CATEGORY_UNSET` is not an unknown number. It is the zero a vendor's
    // `lfx_descriptor d = {0};` leaves behind, so a list of nothing else is a
    // descriptor that named no picture family at all - which has a sentence of
    // its own, and a truer one.
    let stated: Vec<LfxCategory> = declared
        .iter()
        .copied()
        .take(LFX_MAX_CATEGORIES as usize)
        .filter(|category| *category != LFX_CATEGORY_UNSET)
        .collect();
    let mut notes: Vec<LfxRejection> = if stated.is_empty() {
        vec![LfxRejection::NoCategoryDeclared]
    } else {
        stated
            .into_iter()
            .filter(|category| family_of(*category).is_none())
            .collect::<BTreeSet<LfxCategory>>()
            .into_iter()
            .map(|declared| LfxRejection::UnknownCategory { declared })
            .collect()
    };
    let count = count_of(declared.len());
    if count > LFX_MAX_CATEGORIES {
        notes.push(LfxRejection::TooManyCategories { declared: count });
    }
    notes
}

/// The distance a padded region of interest would pad by, or `None` for a
/// declaration that names no usable one.
///
/// **One predicate, read from both ends** (docs/impl/lfx.md §4.1 item 3):
/// [`traits_of`] turns `Some` into [`Roi::PaddedPx`] and `None` into
/// [`Roi::Exact`], and [`notes`] puts [`LfxRejection::PaddingWithoutDistance`]
/// beside exactly the `None` a `PADDED` declaration produced. Written twice
/// they would drift in silence - the schema scheduling a padded region while
/// the report says nothing, or the report naming a fault the schema did not
/// find.
fn padding_of(traits: &Traits) -> Option<f32> {
    (traits.roi_padding_px.is_finite() && traits.roi_padding_px > 0.0)
        .then_some(traits.roi_padding_px)
}

/// A declared count as the ABI's ceilings are written, saturating rather than
/// wrapping: a list too long to count is certainly too long to carry.
fn count_of(values: usize) -> u32 {
    u32::try_from(values).unwrap_or(u32::MAX)
}

/// The source-relative frame offsets a declared window is, leaked for the
/// session. The window that asks for nothing is the constant every built-in
/// carries, so the common case allocates nothing at all.
fn temporal_window(lo: i32, hi: i32) -> &'static [i32] {
    if lo == 0 && hi == 0 {
        return &[0];
    }
    leak_slice((lo..=hi).collect())
}

/// One string, for the session (see the module header).
fn leak(text: &str) -> &'static str {
    Box::leak(text.to_owned().into_boxed_str())
}

/// One list, for the session.
fn leak_slice<T>(values: Vec<T>) -> &'static [T] {
    Box::leak(values.into_boxed_slice())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::describe::{Describe, Identity};
    use lumit_lfx_abi::{
        LfxTraits, LFX_ALPHA_PREMULTIPLIED, LFX_COST_HEAVY, LFX_PARAM_FLAG_HIDDEN,
        LFX_PARAM_STRING, LFX_ROI_UNSET,
    };

    /// The descriptor half of a plugin that is about something else.
    fn identity() -> Identity {
        Identity {
            id: "com.example.blur".to_owned(),
            name: "Example blur".to_owned(),
            vendor: "Example".to_owned(),
            major: 1,
            minor: 2,
            patch: 3,
            categories: vec![LFX_CATEGORY_BLUR_SHARPEN],
            traits: None,
            required_extensions: Vec::new(),
        }
    }

    fn plugin(identity: Identity, described: crate::describe::Described) -> PluginDescriptor {
        PluginDescriptor::new(identity, described)
    }

    fn float(id: &str, unit: Unit) -> Declaration {
        Declaration {
            id: id.to_owned(),
            label: id.to_owned(),
            unit,
            flags: 0,
            kind: Declared::Float {
                default: 0.0,
                slider: (0.0, 1.0),
                hard: (None, None),
            },
        }
    }

    /// A described plugin becomes the rows it declared, in declaration order,
    /// under the name and version the descriptor minted (§14 item 2).
    #[test]
    fn a_described_plugin_becomes_the_rows_it_declared_in_order() {
        let mut sink = Describe::new();
        assert!(sink.declare(float("radius", Unit::Px)));
        assert!(sink.declare(Declaration {
            id: "mode".to_owned(),
            label: "Mode".to_owned(),
            unit: Unit::Raw,
            flags: 0,
            kind: Declared::Choice {
                options: vec!["Box".to_owned(), "Gaussian".to_owned()],
                default: 1,
                dividers_after: vec![0],
            },
        }));
        assert!(sink.declare(Declaration {
            id: "analyse".to_owned(),
            label: "Analyse".to_owned(),
            unit: Unit::Raw,
            flags: 0,
            kind: Declared::Action,
        }));
        let described = sink.finish().expect("nothing structural happened");
        let schema = schema_of(&plugin(identity(), described)).expect("it is an effect");

        assert_eq!(schema.match_name, "lfx:com.example.blur");
        assert_eq!(schema.label, "Example blur");
        assert_eq!(schema.version, 1_002_003, "the three numbers minted one");
        assert_eq!(schema.category, FxCategory::BlurSharpen);
        assert_eq!(schema.matte, MatteRole::None);
        assert!(schema.enabled_when.is_empty());

        let ids: Vec<&str> = schema.params.iter().map(|row| row.id).collect();
        assert_eq!(ids, ["radius", "mode", "analyse"]);
        assert_eq!(schema.params.first().map(|row| row.unit), Some(Unit::Px));
        assert_eq!(
            schema.params.get(1).map(|row| row.kind),
            Some(ParamKind::Choice {
                options: &["Box", "Gaussian"],
                default: 1,
                dividers_after: &[0],
            }),
            "the dividers were declared, not guessed from the labels"
        );
        assert_eq!(
            schema.params.get(2).map(|row| row.kind),
            Some(ParamKind::Action)
        );
    }

    /// A point becomes the two rows the panel folds back into one crosshair -
    /// Lumit has no Point kind, and the suffix rule is what
    /// [`EffectSchema::pairs`] reads.
    #[test]
    fn a_point_becomes_two_rows_the_panel_folds_back() {
        let mut sink = Describe::new();
        assert!(sink.declare(Declaration {
            id: "centre".to_owned(),
            label: "Centre".to_owned(),
            unit: Unit::Px,
            flags: 0,
            kind: Declared::Point2 {
                default: (0.25, 0.75),
                slider: (-1.0, 1.0),
            },
        }));
        let described = sink.finish().expect("nothing structural happened");
        let schema = schema_of(&plugin(identity(), described)).expect("it is an effect");

        let ids: Vec<&str> = schema.params.iter().map(|row| row.id).collect();
        assert_eq!(ids, ["centre_x", "centre_y"]);
        let labels: Vec<&str> = schema.params.iter().map(|row| row.label).collect();
        assert_eq!(labels, ["Centre X", "Centre Y"]);
        assert_eq!(
            schema.params.first().map(|row| row.kind),
            Some(ParamKind::Float {
                default: 0.25,
                slider: (-1.0, 1.0),
                hard: (None, None),
            })
        );
        let pairs: Vec<&'static str> = schema.pairs().map(|pair| pair.stem).collect();
        assert_eq!(pairs, ["centre"], "the panel found no crosshair to fold");
    }

    /// The routes home reverse exactly what the rows were minted from: one
    /// element per declaration that carries a value, a point counted once, an
    /// Action counted not at all (§14 item 2).
    #[test]
    fn value_routes_reverse_exactly_what_the_rows_were_minted_from() {
        let mut sink = Describe::new();
        assert!(sink.declare(float("radius", Unit::Px)));
        assert!(sink.declare(Declaration {
            id: "analyse".to_owned(),
            label: "Analyse".to_owned(),
            unit: Unit::Raw,
            flags: 0,
            kind: Declared::Action,
        }));
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
        assert!(sink.declare(Declaration {
            id: "tint".to_owned(),
            label: "Tint".to_owned(),
            unit: Unit::Raw,
            flags: 0,
            kind: Declared::Colour {
                default: [1.0, 1.0, 1.0, 1.0],
                range: (0.0, 1.0),
            },
        }));
        let described = sink.finish().expect("nothing structural happened");
        let plugin = plugin(identity(), described);
        let schema = schema_of(&plugin).expect("it is an effect");
        let routes = value_routes(&plugin, &schema);

        // Every route names a row the schema actually has, by the id the bag
        // keys it under.
        for route in &routes {
            let row = schema
                .params
                .iter()
                .find(|row| row.id == route.row)
                .expect("a route named a row the schema has not got");
            assert_eq!(route.id, ParamId::new(row.id));
        }
        let spelled: Vec<(&str, u32, usize, usize)> = routes
            .iter()
            .map(|route| (route.row, route.element, route.component, route.dimension))
            .collect();
        assert_eq!(
            spelled,
            [
                ("radius", 0, 0, 1),
                ("centre_x", 1, 0, 2),
                ("centre_y", 1, 1, 2),
                ("tint", 2, 0, 1),
            ],
            "the Action took an element, or the point took two"
        );
        assert_eq!(
            routes.get(1).map(|route| route.kind),
            Some(lumit_lfx_abi::LFX_PARAM_POINT2),
            "a point's axis did not name the arm both halves fold into"
        );
    }

    /// Every row the lowering mints carries its value somewhere, and the four
    /// `ParamKind`s LFX never mints say so rather than being forgotten. The
    /// exhaustive `match` with no `_` arm is the real guard - this is what it
    /// is guarding.
    #[test]
    fn every_param_kind_the_lowering_mints_carries_its_value() {
        for crossing in [
            ParamKind::Float {
                default: 0.0,
                slider: (0.0, 1.0),
                hard: (None, None),
            },
            ParamKind::Slider {
                default: 0.0,
                range: (0.0, 1.0),
                log: false,
            },
            ParamKind::Int {
                default: 0,
                slider: (0, 1),
                hard: (None, None),
            },
            ParamKind::Angle {
                default: 0.0,
                dial_step: 1.0,
            },
            ParamKind::Bool { default: false },
            ParamKind::Choice {
                options: &["One"],
                default: 0,
                dividers_after: &[],
            },
            ParamKind::Colour {
                default: [0.0; 4],
                range: (0.0, 1.0),
            },
            ParamKind::Seed,
            ParamKind::Curve {
                default: &[[0.0, 0.0], [1.0, 1.0]],
            },
            ParamKind::File {
                filter: &["cube"],
                filter_name: "LUTs",
            },
        ] {
            assert!(
                carriage(&crossing).crosses(),
                "{crossing:?} mints a row whose value reaches nothing"
            );
        }
        assert_eq!(carriage(&ParamKind::Action), Carriage::NoValue);
        for unminted in [
            ParamKind::ColourName {
                role: lumit_core::fx::ColourNameRole::Space,
            },
            ParamKind::Layer {
                self_default: false,
            },
            ParamKind::Clip,
            ParamKind::MaskPath {
                self_default: false,
            },
        ] {
            assert_eq!(carriage(&unminted), Carriage::Unminted);
        }
    }

    /// A `memset` trait block schedules as the pessimistic case, and so does a
    /// missing one: heavy, whole input, premultiplied, no neighbours
    /// (§14 item 3, docs/impl/lfx.md §2.4).
    #[test]
    fn a_memset_trait_block_schedules_as_heavy_and_full_frame() {
        let zeroed = traits_of(Some(&Traits::from_abi(&LfxTraits::default())))
            .expect("a zeroed window is [0, 0]");
        assert_eq!(zeroed.cost, CostClass::Heavy);
        assert_eq!(zeroed.roi, Roi::FullFrame);
        assert_eq!(zeroed.temporal, &[0]);
        assert!(zeroed.premultiplied);
        assert!(!zeroed.seeded);

        let absent = traits_of(None).expect("there is nothing to refuse");
        assert_eq!(
            absent, zeroed,
            "a null block read differently from a zeroed one"
        );
    }

    /// The declared traits reach the schema as themselves, and the two fields
    /// that lower onto one tuple variant do so: a padded region and its
    /// distance become [`Roi::PaddedPx`], and a padding with no distance is
    /// exact plus a line in the report.
    #[test]
    fn a_declared_trait_block_lowers_field_by_field() {
        let declared = Traits {
            cost: LFX_COST_CHEAP,
            roi_kind: LFX_ROI_PADDED,
            roi_padding_px: 8.0,
            temporal_lo: -1,
            temporal_hi: 2,
            alpha: LFX_ALPHA_STRAIGHT,
            flags: LFX_TRAIT_SEEDED,
            scratch_bytes_per_megapixel: 0,
        };
        let traits = traits_of(Some(&declared)).expect("the window contains the frame");
        assert_eq!(traits.cost, CostClass::Cheap);
        assert_eq!(traits.roi, Roi::PaddedPx(8.0));
        assert_eq!(traits.temporal, &[-1, 0, 1, 2]);
        assert!(
            !traits.premultiplied,
            "straight alpha was read as premultiplied"
        );
        assert!(traits.seeded);

        let padded = Traits {
            roi_padding_px: 0.0,
            ..declared
        };
        assert_eq!(
            traits_of(Some(&padded)).expect("still an effect").roi,
            Roi::Exact
        );
        let mut identity = identity();
        identity.traits = Some(padded);
        let plugin = plugin(identity, crate::describe::Described::default());
        assert!(notes(&plugin).contains(&LfxRejection::PaddingWithoutDistance));

        // An alpha nobody stated is the premultiplied working form, and a cost
        // nobody stated is heavy - the pessimistic answers, not discriminant
        // nought.
        let unstated = Traits {
            alpha: 0,
            cost: 0,
            roi_kind: LFX_ROI_UNSET,
            ..declared
        };
        let traits = traits_of(Some(&unstated)).expect("still an effect");
        assert!(traits.premultiplied);
        assert_eq!(traits.cost, CostClass::Heavy);
        assert_eq!(traits.roi, Roi::FullFrame);
    }

    /// A temporal window nobody can honour is refused by name rather than
    /// clamped: it is the *gate* the neighbour walk reads, and a clamp would
    /// hand the effect a different window from the one it says it reads.
    #[test]
    fn a_temporal_window_the_host_cannot_honour_is_refused() {
        let base = Traits {
            cost: LFX_COST_HEAVY,
            alpha: LFX_ALPHA_PREMULTIPLIED,
            ..Traits::default()
        };
        for (lo, hi) in [
            (1_i32, 2_i32),
            (-2, -1),
            (-LFX_MAX_TEMPORAL_WINDOW - 1, 0),
            (0, LFX_MAX_TEMPORAL_WINDOW + 1),
        ] {
            assert_eq!(
                traits_of(Some(&Traits {
                    temporal_lo: lo,
                    temporal_hi: hi,
                    ..base
                })),
                Err(LfxRejection::TemporalWindowUnusable { lo, hi }),
                "[{lo}, {hi}] was accepted"
            );
        }
        // Both ends at their limit are in range, and the window is every frame
        // between them.
        let widest = traits_of(Some(&Traits {
            temporal_lo: -LFX_MAX_TEMPORAL_WINDOW,
            temporal_hi: LFX_MAX_TEMPORAL_WINDOW,
            ..base
        }))
        .expect("the declared ceiling is declarable");
        assert_eq!(widest.temporal.len(), 129);
    }

    /// The first declared family is the heading and the rest are keywords, and
    /// a number outside the closed vocabulary lands in Utility with a line in
    /// the report - nothing else changes (§14 item 3).
    #[test]
    fn the_first_declared_family_is_the_heading_and_the_rest_are_keywords() {
        let mut identity = identity();
        identity.categories = vec![LFX_CATEGORY_STYLISE, LFX_CATEGORY_COLOUR, 99];
        let plugin = plugin(identity, crate::describe::Described::default());

        assert_eq!(
            families(&plugin),
            [FxCategory::Stylise, FxCategory::Colour, FxCategory::Utility]
        );
        assert_eq!(
            schema_of(&plugin).expect("it is an effect").category,
            FxCategory::Stylise
        );
        assert_eq!(
            notes(&plugin),
            vec![LfxRejection::UnknownCategory { declared: 99 }]
        );

        let mut nameless = self::identity();
        nameless.categories.clear();
        let nameless = self::plugin(nameless, crate::describe::Described::default());
        assert_eq!(families(&nameless), [FxCategory::Utility]);
        assert_eq!(notes(&nameless), vec![LfxRejection::NoCategoryDeclared]);

        // The ABI's own ceiling, asked here because the header says the host
        // enforces the same numbers where a stranger's bytes arrive: a list
        // longer than the closed vocabulary is read up to the ceiling and the
        // overflow is a line rather than a silence.
        let mut greedy = self::identity();
        greedy.categories = vec![LFX_CATEGORY_UTILITY; LFX_MAX_CATEGORIES as usize + 4];
        greedy.required_extensions =
            vec!["lfx.temporal".to_owned(); LFX_MAX_REQUIRED_EXTENSIONS as usize + 1];
        let greedy = self::plugin(greedy, crate::describe::Described::default());
        assert_eq!(families(&greedy), [FxCategory::Utility]);
        assert_eq!(
            notes(&greedy),
            vec![
                LfxRejection::TooManyCategories {
                    declared: LFX_MAX_CATEGORIES + 4
                },
                LfxRejection::TooManyRequiredExtensions {
                    declared: LFX_MAX_REQUIRED_EXTENSIONS + 1
                },
            ]
        );
        // And the list really is read up to the ceiling, which is what the
        // line beside it says happened: the sentence and the list are read
        // from one place, so an overflow named is an overflow dropped.
        assert_eq!(
            required_extensions(&greedy).len(),
            LFX_MAX_REQUIRED_EXTENSIONS as usize,
            "the overflow was named and kept"
        );
        assert_eq!(
            required_extensions(&self::plugin(
                self::identity(),
                crate::describe::Described::default()
            )),
            &[] as &[String],
            "a list under the ceiling was cut"
        );

        // A number outside the vocabulary declared three times is one mistake,
        // so it is one sentence: `families` drops the repeats and the notes
        // beside them are dropped the same way.
        let mut repeated = self::identity();
        repeated.categories = vec![99, 99, 99];
        let repeated = self::plugin(repeated, crate::describe::Described::default());
        assert_eq!(families(&repeated), [FxCategory::Utility]);
        assert_eq!(
            notes(&repeated),
            vec![LfxRejection::UnknownCategory { declared: 99 }],
            "the same sentence was printed once per repeat"
        );

        // `LFX_CATEGORY_UNSET` is the zero a vendor left behind rather than a
        // number from a newer header, so a list of nothing else is the
        // descriptor that named no family at all - and beside a real family it
        // claims nothing and adds no keyword.
        let mut unset = self::identity();
        unset.categories = vec![LFX_CATEGORY_UNSET, LFX_CATEGORY_UNSET];
        let unset = self::plugin(unset, crate::describe::Described::default());
        assert_eq!(families(&unset), [FxCategory::Utility]);
        assert_eq!(notes(&unset), vec![LfxRejection::NoCategoryDeclared]);

        let mut half = self::identity();
        half.categories = vec![LFX_CATEGORY_COLOUR, LFX_CATEGORY_UNSET];
        let half = self::plugin(half, crate::describe::Described::default());
        assert_eq!(families(&half), [FxCategory::Colour]);
        assert!(notes(&half).is_empty());
    }

    /// The descriptor's own three strings meet the ceiling every row's strings
    /// meet, and meet it where they arrive.
    ///
    /// The sink asks it of a row's id, label and option text because the header
    /// says the host enforces the same numbers where a stranger's bytes arrive;
    /// the descriptor's id, name and vendor arrive here instead, since a
    /// `PluginDescriptor` need not have come through a sink at all. Structural
    /// rather than a line: a row this build cannot draw costs that one control,
    /// while an id nobody can carry is the match name a project file would have
    /// stored.
    #[test]
    fn a_descriptors_own_strings_meet_the_same_ceiling() {
        let long = "x".repeat(lumit_lfx_abi::LFX_MAX_STRING_BYTES as usize);
        for (field, set) in [
            (
                "id",
                (|id: &mut Identity, text: String| id.id = text) as fn(&mut Identity, String),
            ),
            ("name", |id: &mut Identity, text: String| id.name = text),
            ("vendor", |id: &mut Identity, text: String| id.vendor = text),
        ] {
            let mut over = identity();
            set(&mut over, long.clone());
            assert_eq!(
                schema_of(&plugin(over, crate::describe::Described::default())),
                Err(LfxRejection::IdentityStringTooLong {
                    field: field.to_owned(),
                    bytes: lumit_lfx_abi::LFX_MAX_STRING_BYTES + 1,
                }),
                "the descriptor's {field} was leaked at whatever length it arrived"
            );
        }

        // Right up against the ceiling is an effect: the refusal is for what
        // goes past it.
        let mut snug = identity();
        snug.name = "x".repeat(lumit_lfx_abi::LFX_MAX_STRING_BYTES as usize - 1);
        assert!(schema_of(&plugin(snug, crate::describe::Described::default())).is_ok());
    }

    /// A control that cannot carry a unit is in none, whatever it declared, and
    /// an angle is in degrees by definition - the default `#[derive(Effect)]`
    /// reaches for when a built-in's author says nothing, applied here as a
    /// **normalisation with a line in the report**: the header says an angle's
    /// unit must be `LFX_UNIT_DEGREES`, and an author whose declaration was
    /// rewritten is owed the sentence saying so.
    #[test]
    fn a_row_is_in_the_unit_its_kind_allows() {
        assert_eq!(unit_of(LFX_UNIT_PX), Unit::Px);
        assert_eq!(unit_of(0), Unit::Unset, "unset is not a unit");
        assert_eq!(unit_of(9_999), Unit::Unset, "a number from a newer header");

        let mut sink = Describe::new();
        assert!(sink.declare(Declaration {
            id: "rotation".to_owned(),
            label: "Rotation".to_owned(),
            unit: Unit::Seconds,
            flags: 0,
            kind: Declared::Angle {
                default: 0.0,
                dial_step: 15.0,
            },
        }));
        assert!(sink.declare(Declaration {
            id: "invert".to_owned(),
            label: "Invert".to_owned(),
            unit: Unit::Px,
            flags: 0,
            kind: Declared::Bool { default: false },
        }));
        // Declared in the unit its kind is in anyway: normalising it changes
        // nothing, so there is nothing to report.
        assert!(sink.declare(Declaration {
            id: "invert_too".to_owned(),
            label: "Invert too".to_owned(),
            unit: Unit::Raw,
            flags: 0,
            kind: Declared::Bool { default: false },
        }));
        assert!(sink.declare(float("reach", Unit::Frames)));
        let described = sink.finish().expect("nothing structural happened");
        assert_eq!(
            described.report,
            vec![
                LfxRejection::UnitIgnoredForKind {
                    id: "rotation".to_owned(),
                    declared: Unit::Seconds,
                },
                LfxRejection::UnitIgnoredForKind {
                    id: "invert".to_owned(),
                    declared: Unit::Px,
                },
            ],
            "a declaration the lowering overrode went unreported"
        );
        let schema = schema_of(&plugin(identity(), described)).expect("it is an effect");

        let units: Vec<Unit> = schema.params.iter().map(|row| row.unit).collect();
        assert_eq!(units, [Unit::Degrees, Unit::Raw, Unit::Raw, Unit::Frames]);
    }

    /// A heading becomes a contiguous run of rows the panel tucks behind one
    /// twirl, and a point inside it contributes both of its rows to that run.
    #[test]
    fn a_heading_becomes_one_run_of_the_rows_declared_inside_it() {
        let mut sink = Describe::new();
        assert!(sink.declare(float("amount", Unit::Percent)));
        assert!(sink.group_begin("advanced", "Advanced", 0));
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
        assert!(sink.declare(float("falloff", Unit::Raw)));
        assert!(sink.group_end());
        let described = sink.finish().expect("nothing structural happened");
        let schema = schema_of(&plugin(identity(), described)).expect("it is an effect");

        assert_eq!(schema.groups.len(), 1);
        let group = schema.groups.first().expect("the heading");
        assert_eq!(group.label, "Advanced");
        assert_eq!(group.params, ["centre_x", "centre_y", "falloff"]);
        assert!(!group.collapsed);
    }

    /// A hidden row and a hidden heading are both hidden, and the rows stay in
    /// the schema either way: a row that starts hidden has to be there to
    /// appear later.
    #[test]
    fn a_hidden_row_and_a_hidden_heading_are_both_hidden() {
        let mut sink = Describe::new();
        assert!(sink.declare(float("shown", Unit::Raw)));
        let mut secret = float("secret", Unit::Raw);
        secret.flags = LFX_PARAM_FLAG_HIDDEN;
        assert!(sink.declare(secret));
        assert!(sink.group_begin("later", "Later", LFX_PARAM_FLAG_HIDDEN));
        assert!(sink.declare(float("inside", Unit::Raw)));
        assert!(sink.group_end());
        let described = sink.finish().expect("nothing structural happened");
        let plugin = plugin(identity(), described);

        let schema = schema_of(&plugin).expect("it is an effect");
        assert_eq!(schema.params.len(), 3, "a hidden row left the schema");
        assert_eq!(
            hidden_rows(&plugin, &schema),
            ["inside", "secret"].into_iter().collect::<BTreeSet<&str>>()
        );
    }

    /// A release the frame key cannot tell apart from another one refuses the
    /// effect at the lowering, where the whole of the descriptor is in hand
    /// (§14 item 3).
    #[test]
    fn a_version_outside_the_injective_range_refuses_the_effect() {
        let mut identity = identity();
        identity.patch = 1_000;
        assert_eq!(
            schema_of(&plugin(identity, crate::describe::Described::default())),
            Err(LfxRejection::VersionOutOfRange {
                major: 1,
                minor: 2,
                patch: 1_000,
            })
        );
    }

    /// The duplicate check is asked again at the lowering, because a descriptor
    /// need not have come through a sink at all - a proto message, or a test.
    #[test]
    fn two_rows_on_one_param_id_refuse_the_effect_at_the_lowering() {
        let described = crate::describe::Described {
            params: vec![float("gain", Unit::Raw), float("gain", Unit::Raw)],
            groups: Vec::new(),
            report: Vec::new(),
        };
        assert_eq!(
            schema_of(&plugin(identity(), described)),
            Err(LfxRejection::DuplicateParamId {
                first: "gain".to_owned(),
                second: "gain".to_owned(),
            })
        );
    }

    /// The scan report is the sink's lines followed by the descriptor's own, so
    /// one plugin's whole story is one list. **Not** the order the faults
    /// happened in: the descriptor's category list was read before `describe`
    /// ran at all, so the family the plugin declared first is the line the page
    /// prints last.
    #[test]
    fn the_report_carries_the_sinks_lines_and_the_lowerings() {
        let mut sink = Describe::new();
        assert!(!sink.decline_kind(LFX_PARAM_STRING, "caption"));
        assert!(sink.declare(float("gain", Unit::Raw)));
        let described = sink.finish().expect("a text row does not end the effect");

        let mut identity = identity();
        identity.categories = vec![7_777];
        let plugin = plugin(identity, described);
        assert_eq!(
            notes(&plugin),
            vec![
                LfxRejection::NoTextRow {
                    id: "caption".to_owned()
                },
                LfxRejection::UnknownCategory { declared: 7_777 },
            ]
        );
        assert!(
            notes(&plugin).iter().all(|why| !why.refuses_the_effect()),
            "a report line ended the effect"
        );
        assert!(schema_of(&plugin).is_ok(), "the plugin did not still load");
    }
}
