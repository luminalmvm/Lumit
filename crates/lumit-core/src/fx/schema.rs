/// Cost class (docs/08 §1.3) — consumed by degradation ordering and budgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostClass {
    Trivial,
    Cheap,
    Moderate,
    Heavy,
}

/// Region-of-interest support (docs/08 §1.3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Roi {
    /// Output pixel needs only the same input pixel.
    Exact,
    /// Needs input dilated by a radius, in % of the comp diagonal (§2.3).
    PaddedPctDiag(f32),
    /// Needs the whole input.
    FullFrame,
}

/// Static trait declaration (docs/08 §1.3), read by the scheduler and caches.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectTraits {
    pub cost: CostClass,
    pub roi: Roi,
    /// Source-relative frame offsets required; `&[0]` = current frame only.
    pub temporal: &'static [i32],
    /// True = operates on premultiplied alpha (the default working form).
    pub premultiplied: bool,
    pub seeded: bool,
    pub beat_input: bool,
}

/// One declared parameter (docs/08 §1.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamSchema {
    /// Stable snake_case identifier (expressions address this).
    pub id: &'static str,
    /// Sentence-case UI label.
    pub label: &'static str,
    pub kind: ParamKind,
}

/// Parameter type + defaults/ranges (docs/08 §1.2: sliders may be exceeded
/// by typing; hard ranges may not).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParamKind {
    Float {
        default: f64,
        slider: (f64, f64),
        /// Hard bounds; either side may be None (K-090: a threshold clamps
        /// at zero below and runs unbounded above).
        hard: (Option<f64>, Option<f64>),
    },
    /// A whole-number parameter (a blade count, a ghost cap). The VALUE side
    /// is still an `EffectValue::Float` — it animates and serialises exactly
    /// like a Float — the kind only tells the UI to step and display it as an
    /// integer and the resolve step to round it, replacing the old "rounded
    /// float row" convention (docs/08 §1.2).
    Int {
        default: i64,
        slider: (i64, i64),
        /// Hard bounds; either side may be None, matching Float.
        hard: (Option<i64>, Option<i64>),
    },
    /// An angle in degrees — the parameter type docs/08 §1.1 has listed since
    /// the beginning and docs/07-UI-SPEC.md §6 names among the widgets, drawn
    /// as a **dial** beneath its number rather than a slider.
    ///
    /// The stored value is an ordinary [`EffectValue::Float`](crate::model::
    /// EffectValue::Float), so keyframes, expressions and the resolve step see
    /// no new shape: an angle *is* a number of degrees, and the only thing this
    /// kind changes is the control drawn for it. Deliberately unbounded — an
    /// angle animates through full turns rather than stopping at 360, which is
    /// why the existing degree parameters (Shake's rotation, the aperture's
    /// blade rotation) declare `hard: (None, None)`. `dial_step` is the
    /// snapping increment while a modifier is held, in degrees.
    ///
    /// There is deliberately **no `Point` kind beside this one**. A 2-D point
    /// is already a pair of adjacent `_x`/`_y` Floats that the panel folds into
    /// one row with a crosshair pick (docs/07 §6.1) — the Lens flare's Light
    /// and Radial blur's Centre both ride it — so a point needs no schema kind
    /// of its own, only the naming convention. An angle has no such fallback:
    /// there is no arrangement of existing rows that draws a dial.
    Angle {
        default: f64,
        dial_step: f64,
    },
    Choice {
        options: &'static [&'static str],
        default: u32,
        /// Option indices after which the dropdown draws a group divider
        /// (T21): e.g. Echo's Mode lists its effect-only compositing orders
        /// (Behind / In front) first, then a divider, then the standard blend
        /// modes. Empty for an ungrouped list. The [`CHOICE_UNGROUPED`] alias
        /// spells "no dividers" for the common case.
        dividers_after: &'static [u32],
    },
    Bool {
        default: bool,
    },
    Colour {
        /// Scene-linear RGBA (docs/08 §1.1's colour type); channels animate
        /// independently in the model.
        default: [f64; 4],
        /// Per-channel edit range — linear values may exceed 1 (HDR tints)
        /// or dip below 0 (a lift), so each colour declares its own.
        range: (f64, f64),
    },
    /// An integer seed (docs/08 §1.1's seed type): selects which
    /// deterministic random-looking sequence a seeded effect follows
    /// (§2.4). No declared default — the default is per-instance (§3.4),
    /// drawn from the fresh instance id at instantiation, so two copies of
    /// a seeded effect never wobble in sync.
    Seed,
    /// A file path chosen from a dialog (K-111), e.g. a `.cube` LUT. The
    /// `filter` extensions (lower-case, no dot) and `filter_name` drive the
    /// open dialog. The value carries a [`FileParam`]; it animates only by
    /// stepping (hold keys), since two paths cannot be blended.
    File {
        filter: &'static [&'static str],
        filter_name: &'static str,
    },
    /// A reference to another layer in the composition (docs/impl/
    /// layer-input.md), sampled as an auxiliary picture — the depth pass a
    /// depth-of-field effect reads, the bright-source matte a Lens flare
    /// detects lights in. The value carries an [`EffectValue::Layer`] (an
    /// optional layer id); the caller renders that layer alone at comp size
    /// and threads its texture beside the resolved op, exactly as a matte
    /// layer is rendered alone. Unset (or a dangling reference) is a
    /// labelled no-op, never a fault.
    ///
    /// **This layer** (K-288): a reference to the layer the effect is *on*
    /// is not a re-render of that layer — it is the effect's own input at
    /// its point in the stack. On an ordinary layer that is the picture the
    /// effect is about to process; on an **adjustment layer** it is the
    /// composite of everything below, which is the only thing an adjustment
    /// layer has to look at. `self_default` declares that a fresh instance
    /// should start pointed at its own layer (the Lens flare's matte, whose
    /// natural reading is "the lights in this picture"), rather than unset.
    Layer {
        /// A fresh instance added to a layer starts referencing that layer
        /// (see the type docs). `false` leaves it unset, the historical
        /// no-op default — a depth pass is never the picture itself.
        self_default: bool,
    },
}

/// How a transform- or displacement-domain effect treats the border pixels
/// its warp reveals (P3, K-145): the one reusable Edges control, shared by
/// the blur family (docs/08 §3.8) and Shake (§3.4). The `u32` codes are the
/// wire form the resolved ops and every WGSL kernel read — 0 Transparent,
/// 1 Repeat, 2 Mirror — so the enum only names those numbers, it never
/// changes them. Any effect whose resample can pull in area outside the
/// frame reuses this rather than re-deciding what an edge means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgesMode {
    /// Revealed area is transparent (contributes nothing, full weight kept).
    Transparent,
    /// The border pixel is held outward (clamp-to-edge).
    Repeat,
    /// The image reflects at the border, without repeating the edge sample.
    Mirror,
}

impl EdgesMode {
    /// The Choice option labels, in code order (index 0/1/2). A schema's Edges
    /// parameter declares `options: EdgesMode::OPTIONS` (aliased as the shared
    /// [`EDGE_OPTIONS`](crate::fx::EDGE_OPTIONS) const the blur family already
    /// uses).
    pub const OPTIONS: &'static [&'static str] = &["Transparent", "Repeat", "Mirror"];

    /// The wire code the resolved ops and the WGSL kernels read.
    pub const fn code(self) -> u32 {
        match self {
            EdgesMode::Transparent => 0,
            EdgesMode::Repeat => 1,
            EdgesMode::Mirror => 2,
        }
    }

    /// The mode for a stored Choice index, or `None` for an unknown code (a
    /// caller supplies its own default). 0 Transparent, 1 Repeat, 2 Mirror.
    pub const fn from_code(code: u32) -> Option<Self> {
        match code {
            0 => Some(EdgesMode::Transparent),
            1 => Some(EdgesMode::Repeat),
            2 => Some(EdgesMode::Mirror),
            _ => None,
        }
    }
}

/// A collapsible group of parameters inside one effect's parameter list
/// (P4, K-145): the disclosure "twirl" the Effect Controls draws so an effect
/// can tuck advanced controls behind a header (Shake's per-axis wobble). The
/// group is driven entirely from schema metadata, so any effect adopts it by
/// declaring one in its [`EffectSchema::groups`]; the UI renders the named
/// params under `label` and hides them when the twirl is closed. The member
/// ids must be a contiguous run in the schema's `params` (they render in
/// place, where the group's first member sits).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamGroup {
    /// Sentence-case disclosure header. An EMPTY label renders headerless:
    /// the member rows appear in place with no twirl — the shape a
    /// conditional run of parameters wants (the Lens flare's matte rows).
    pub label: &'static str,
    /// The member parameter ids, naming params in the same schema.
    pub params: &'static [&'static str],
    /// Whether the twirl starts closed (the advanced-by-default case).
    pub collapsed: bool,
    /// When set, the whole group is shown only while the named sibling
    /// Choice parameter holds one of the given indices — how an effect's
    /// panel offers different controls per mode (the Lens flare's Source
    /// type: its matte rows answer to Matte alone, its source-colour toggle
    /// to Matte *and* Lights). None, or an empty set, is always visible.
    pub visible_when: Option<(&'static str, &'static [u32])>,
    /// When set, the group is shown only while the lens in play has at least
    /// this many glass elements (K-371).
    ///
    /// The Lens flare offers a coating choice per element, and lenses have
    /// between four and eighteen of them, so the row count has to follow the
    /// lens rather than being fixed by the schema. Each element's row is its
    /// own single-member group carrying its own threshold, and the effect
    /// draws exactly as many as the chosen lens has.
    ///
    /// **It never crosses the bridge as itself.** A group's visibility is
    /// already resolved in the panel from a live sibling value, so the bridge
    /// turns this into precisely that: the sibling is the Lens dropdown and
    /// the values are the lens indices whose prescription has at least this
    /// many elements, worked out from the library at the time the panel asks.
    /// One mechanism in the frontend, not two.
    ///
    /// Recorded limit: a user's own `.lens` file overrides the dropdown, and
    /// only the file knows its element count, so the rows offered then follow
    /// the *picked* lens. An element with no row keeps the file's own coating,
    /// which is what an untouched row does anyway.
    pub visible_when_lens_elements: Option<u32>,
}

/// One parameter's availability depending on another's value: the greyed-out
/// row every host draws when a control has been taken over by a switch beside
/// it.
///
/// **In plain terms.** Some controls stop meaning anything once another control
/// is set a certain way. Tick "Use focus point" and the focus *distance* number
/// is no longer what decides focus — the point is — so leaving the number live
/// would invite you to drag something that does nothing. Every editor's answer
/// is the same: draw the row greyed and refuse the edit, so the panel tells you
/// which of the two is in charge.
///
/// It is declared on the [`EffectSchema`], not inside [`ParamSchema`], for the
/// same reason [`ParamGroup`] is: it names parameters rather than living in
/// one, and putting it here leaves the 130-odd existing parameter literals
/// untouched. The rule is a *UI affordance and a piece of documentation* — the
/// resolve step implements the real branch itself and never consults this, so a
/// schema that forgets a rule renders correctly and merely draws a live control
/// that does nothing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnabledWhen {
    /// The parameter whose row this governs — the one that greys out.
    pub param: &'static str,
    /// The parameter whose value decides it.
    pub on: &'static str,
    /// What `on` must hold for `param` to be editable.
    pub cond: EnabledCond,
}

/// The condition half of an [`EnabledWhen`].
///
/// Deliberately a small closed set rather than an expression language: these
/// are panel affordances, and every case the built-ins have wanted so far is
/// "a switch is on", "a dropdown is on some entry", or "a layer has been
/// picked". Add a variant when an effect genuinely needs one, and give it a
/// test — do not grow this into a scripting surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EnabledCond {
    /// Editable while the named [`ParamKind::Bool`] holds this value.
    BoolIs(bool),
    /// Editable while the named [`ParamKind::Choice`] is on this option index.
    ChoiceIs(u32),
    /// Editable while the named [`ParamKind::Choice`] is on anything *but*
    /// this option index — the "…unless it is set to None" shape.
    ChoiceIsNot(u32),
    /// Editable while the named [`ParamKind::Layer`] actually names a layer.
    /// An unset or dangling reference greys the dependent row.
    LayerSet,
}

/// The Add-effect menu's grouping (K-090): every schema declares one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FxCategory {
    BlurSharpen,
    Colour,
    Distortion,
    Stylise,
    Temporal,
    Utility,
}

impl FxCategory {
    /// Sentence-case menu label.
    pub const fn label(self) -> &'static str {
        match self {
            FxCategory::BlurSharpen => "Blur & sharpen",
            FxCategory::Colour => "Colour",
            FxCategory::Distortion => "Distortion",
            FxCategory::Stylise => "Stylise",
            FxCategory::Temporal => "Temporal",
            FxCategory::Utility => "Utility",
        }
    }

    /// Every category, in menu order.
    pub const ALL: [FxCategory; 6] = [
        FxCategory::BlurSharpen,
        FxCategory::Colour,
        FxCategory::Distortion,
        FxCategory::Stylise,
        FxCategory::Temporal,
        FxCategory::Utility,
    ];
}

/// One built-in effect's full declaration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectSchema {
    /// Stable match name (participates in the cache key with `version`).
    pub match_name: &'static str,
    pub label: &'static str,
    pub version: u32,
    pub category: FxCategory,
    pub traits: EffectTraits,
    pub params: &'static [ParamSchema],
    /// Collapsible parameter groups (P4, K-145): each names a contiguous run
    /// of `params` the Effect Controls tucks behind a twirl. Empty for the
    /// effects that declare none.
    pub groups: &'static [ParamGroup],
    /// Rows that grey out while another parameter says so. Empty for the
    /// effects whose controls are all independent, which is most of them.
    pub enabled_when: &'static [EnabledWhen],
}
