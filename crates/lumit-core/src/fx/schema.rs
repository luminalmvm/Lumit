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
    Choice {
        options: &'static [&'static str],
        default: u32,
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
    /// depth-of-field effect reads. The value carries an
    /// [`EffectValue::Layer`] (an optional layer id); the caller renders that
    /// layer alone at comp size and threads its texture beside the resolved
    /// op, exactly as a matte layer is rendered alone. Unset (or a dangling
    /// reference) is a labelled no-op, never a fault.
    Layer {},
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
    /// Sentence-case disclosure header.
    pub label: &'static str,
    /// The member parameter ids, naming params in the same schema.
    pub params: &'static [&'static str],
    /// Whether the twirl starts closed (the advanced-by-default case).
    pub collapsed: bool,
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
}
