//! The import report (docs/11-AE-IMPORT.md §9): what the importer did, item by
//! item, in the four grades the spec names.
//!
//! In plain terms: an import never fails and never stops to ask a question, so
//! the only way a person finds out that a blend mode had no equivalent, or that
//! an effect came across as an inert placeholder, is this list. Every row names
//! where it happened (composition → layer → property), what grade the outcome
//! got, and why in one line.
//!
//! The report is **data first, prose second** (docs/impl/ae-import.md §4). A
//! [`Reason`] is a typed enum carrying the facts — the AE name that had no
//! counterpart, the stretch percentage, the pixel aspect — and its [`Display`]
//! turns it into the sentence a panel shows. Nothing in the engine parses those
//! sentences, so re-wording one is never a behaviour change, and a frontend that
//! wants its own phrasing (or its own language) matches on the variant instead.
//!
//! [`Display`]: std::fmt::Display

use serde::{Deserialize, Serialize};

/// Where a row happened. Every part is optional because rows are raised at
/// every depth: a whole item can be skipped before any comp is known, and a
/// comp-wide adjustment names no layer.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ItemPath {
    /// The composition's name, or the project item's for an item-level row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    /// The property path within the layer, as a reader would say it —
    /// "Position", "Mask 1", "Gaussian Blur ▸ Blurriness".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub property: Option<String>,
}

impl ItemPath {
    /// A row about a project item or a whole composition.
    #[must_use]
    pub fn item(name: &str) -> Self {
        Self {
            comp: Some(name.to_string()),
            ..Self::default()
        }
    }

    /// The same path, narrowed to one layer.
    #[must_use]
    pub fn layer(&self, name: &str) -> Self {
        Self {
            comp: self.comp.clone(),
            layer: Some(name.to_string()),
            property: None,
        }
    }

    /// The same path, narrowed to one property of the layer.
    #[must_use]
    pub fn property(&self, name: &str) -> Self {
        Self {
            comp: self.comp.clone(),
            layer: self.layer.clone(),
            property: Some(name.to_string()),
        }
    }
}

impl std::fmt::Display for ItemPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let parts = [
            self.comp.as_deref(),
            self.layer.as_deref(),
            self.property.as_deref(),
        ];
        let mut first = true;
        for part in parts.into_iter().flatten() {
            if !first {
                write!(f, " ▸ ")?;
            }
            write!(f, "{part}")?;
            first = false;
        }
        if first {
            write!(f, "Project")?;
        }
        Ok(())
    }
}

/// The four grades of docs/11 §4 and §9.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Came across whole. Rows are not raised for these — the count is.
    Imported,
    /// Mapped with a documented difference.
    Adjusted,
    /// An inert node keeping the data and the slot; renders as identity.
    Placeholder,
    /// Could not be represented at all; named here rather than lost quietly.
    Skipped,
}

/// Why a row exists. One variant per documented difference, carrying the facts
/// rather than a sentence (see the module note).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "reason")]
pub enum Reason {
    // --- items and compositions ---
    /// An item the walker recorded with no usable id or kind.
    ItemUnreadable,
    /// A composition item with no matching `comps[]` entry: it imports empty
    /// so everything referencing it still resolves.
    CompMissing,
    /// The comp's frame rate was absent or nonsensical; a default was used.
    CompFrameRateGuessed { used: f64 },
    /// The comp's duration was absent or non-positive; a default was used.
    CompDurationGuessed { used: f64 },
    /// Lumit compositions have square pixels (docs/03): a non-1.0 pixel aspect
    /// ratio is recorded in the `ae` namespace and not applied.
    PixelAspectIgnored { par: f64 },
    /// After Effects can start a comp's timecode somewhere other than zero.
    /// Lumit's compositions begin at zero, so the picture is unchanged and the
    /// readout is not.
    CompStartIgnored { start: f64 },
    /// "Preserve frame rate / resolution when nested" have no Lumit switch:
    /// a nested comp is rendered at the parent's rate and raster.
    NestedPreserveIgnored { fps: bool, resolution: bool },
    /// The project composited in 8 bits without linear blending, which is a
    /// different arithmetic from Lumit's scene-linear one (docs/11 §3), so
    /// blend results can shift subtly.
    ProjectBlendingDiffers { bits: u32 },
    /// A renderer this build does not recognise as one whose comps import
    /// flat — the C4D case docs/11 §4 flags prominently.
    RendererUnrecognised { renderer: String },
    /// The footage file was flagged missing by After Effects.
    MediaMissing { path: String },
    /// The item was an After Effects placeholder, not a real file.
    MediaPlaceholder,

    // --- layers ---
    /// A layer the walker recorded with no stacking index: it cannot be
    /// placed, parented to, or used as a matte, so it is the one layer the
    /// import skips.
    LayerUnreadable,
    /// A layer kind with no Lumit counterpart; it keeps its slot as a Null.
    LayerKindUnsupported { ae_kind: String },
    /// The layer named a source item that is not in the capture; it keeps its
    /// slot, its transform and its parenting as a Null.
    LayerSourceMissing { id: i64 },
    /// The layer's bar was empty or back to front in the capture; it imports
    /// one frame long rather than as an invalid span.
    LayerSpanRepaired,
    /// An audio layer, which Lumit carries as an ordinary Footage layer
    /// (docs/01 §2 — the audio channel of footage).
    AudioLayerAsFootage,
    /// A guide layer: Lumit has no guide switch, so the layer imports visible.
    GuideLayerNotSupported,
    /// "Preserve underlying transparency" has no Lumit switch yet.
    PreserveTransparencyNotSupported,
    /// Draft/wireframe layer quality: Lumit renders one quality.
    LayerQualityIgnored { quality: String },
    /// AE's time stretch has no Lumit switch; it imports as the equivalent
    /// Retime, which is the same mapping from layer time to source time.
    StretchAsRetime { percent: f64 },
    /// Pixel Motion frame blending maps to Flow interpolation, but the flow
    /// engine is not AE's, so in-betweens differ (docs/11 §4).
    FlowEngineDiffers,
    /// The layer named a parent that is not in this composition.
    ParentMissing { index: u32 },
    /// The matte named a layer that is not in this composition.
    MatteTargetMissing { index: u32 },
    /// A blend mode with no Lumit equivalent; imported as Normal.
    BlendModeUnavailable { ae_mode: String },
    /// One of AE's "Classic" blend modes; imported as its modern counterpart.
    BlendModeClassic { ae_mode: String },
    /// A shape layer: the paths, fills and strokes are a later stage, so the
    /// layer keeps its slot, transform and parenting but draws nothing.
    ShapeContentsNotMapped,
    /// A text layer whose styling beyond size and fill colour has no home yet.
    TextStylingNotMapped,
    /// A light kind Lumit does not have; imported as the nearest.
    LightKindApproximated { ae_kind: String },

    // --- properties and keyframes ---
    /// The property is spatial in AE (a motion path with tangents); Lumit
    /// animates each axis on its own, so the tangents are not carried.
    SpatialTangentsFlattened,
    /// An expression came across as source text and now drives the property.
    ExpressionCarried,
    /// A disabled expression: the text is kept in the `ae` namespace, and the
    /// keyframes or the still value drive the property, exactly as in AE.
    ExpressionDisabledCarried,
    /// A property After Effects itself could not read (a `CUSTOM_VALUE`
    /// blob — K-410).
    PropertyUnreadable { match_name: String },

    // --- masks ---
    /// Lighten and Darken mask modes are not built (docs/06 §2); imported as
    /// Add.
    MaskModeUnavailable { ae_mode: String },
    /// AE feathers a mask separately in x and y; Lumit has one width.
    MaskFeatherAxesDiffer { x: f64, y: f64 },
    /// A RotoBezier mask: AE computes its tangents rather than storing them,
    /// so the imported path is the polygon through its vertices.
    MaskRotoBezierFlattened,

    // --- effects ---
    /// No mapping for this match name yet, so the instance is inert and keeps
    /// its complete dump (docs/11 §6).
    EffectPlaceholder { match_name: String },
    /// A mapped effect's control that Lumit has no counterpart for. The effect
    /// still imported (docs/11 §5's "reported rather than approximated"); this
    /// one dial did not come with it.
    EffectParamNotCarried { effect: String, param: String },
    /// A mapped effect's control that came across as the nearest thing Lumit
    /// has — an option collapsed onto a shorter list, two radii averaged into
    /// one, a mask reference that fell back to the first mask.
    EffectParamApproximated {
        effect: String,
        param: String,
        imported_as: String,
    },
    /// The effect mapped whole, and evaluates differently by construction:
    /// scene-linear arithmetic where After Effects had eight bits, Lumit's own
    /// field where After Effects' was undocumented. docs/11 §5's "look for
    /// look", said in the report rather than only in the spec.
    EffectDiffers { effect: String, detail: String },
    /// An After Effects control that read the clock — Radio Waves' own time,
    /// Ripple's Wave Speed — became keyframes on an ordinary Lumit control,
    /// which is the same motion and is deterministic (docs/08 §2.4).
    EffectSpeedAsKeyframes { effect: String, param: String },
    /// A placeholder the table deliberately does not map, with the Lumit
    /// feature that does the job instead.
    EffectSuggestion { match_name: String, instead: String },
    /// The control mapped exactly and its *number* changed, because the two
    /// sides measure in different bases: After Effects' raster pixels or per
    /// cents of the layer against Lumit's px@comp and per cents of the comp
    /// diagonal (docs/08 §2.3). Nothing was approximated — the same length is
    /// spelled differently.
    EffectParamRebased { effect: String, param: String },
}

impl std::fmt::Display for Reason {
    #[allow(clippy::too_many_lines)]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ItemUnreadable => write!(f, "the item could not be read and was skipped"),
            Self::CompMissing => write!(f, "no composition data arrived — imported empty"),
            Self::CompFrameRateGuessed { used } => {
                write!(f, "no frame rate arrived — imported at {used} fps")
            }
            Self::CompDurationGuessed { used } => {
                write!(f, "no duration arrived — imported {used} seconds long")
            }
            Self::PixelAspectIgnored { par } => write!(
                f,
                "pixel aspect ratio {par} has no equivalent — imported with square pixels"
            ),
            Self::CompStartIgnored { start } => write!(
                f,
                "the composition's timecode started at {start} seconds — imported starting at zero"
            ),
            Self::NestedPreserveIgnored { fps, resolution } => {
                let what = match (fps, resolution) {
                    (true, true) => "frame rate and resolution",
                    (true, false) => "frame rate",
                    _ => "resolution",
                };
                write!(
                    f,
                    "preserve {what} when nested has no equivalent — nested comps follow the \
                     parent"
                )
            }
            Self::ProjectBlendingDiffers { bits } => write!(
                f,
                "the project blended at {bits} bits without linear blending — Lumit composites in \
                 scene-linear light, so blends can shift slightly"
            ),
            Self::RendererUnrecognised { renderer } => write!(
                f,
                "the {renderer} renderer's features do not import — layers arrive flat"
            ),
            Self::MediaMissing { path } => {
                write!(f, "the file at {path} was missing in After Effects too")
            }
            Self::MediaPlaceholder => write!(f, "an After Effects placeholder, not a real file"),
            Self::LayerUnreadable => {
                write!(f, "the layer had no place in the stack and was skipped")
            }
            Self::LayerKindUnsupported { ae_kind } => write!(
                f,
                "layer kind {ae_kind} has no equivalent — imported as a null so parenting survives"
            ),
            Self::LayerSourceMissing { id } => write!(
                f,
                "the source item {id} is not in the capture — imported as a null so parenting \
                 survives"
            ),
            Self::LayerSpanRepaired => write!(
                f,
                "the layer's in and out points did not make a span — imported one frame long"
            ),
            Self::AudioLayerAsFootage => {
                write!(f, "imported as a footage layer carrying its audio")
            }
            Self::GuideLayerNotSupported => {
                write!(f, "guide layers have no equivalent — imported visible")
            }
            Self::PreserveTransparencyNotSupported => write!(
                f,
                "preserve underlying transparency has no equivalent yet — not applied"
            ),
            Self::LayerQualityIgnored { quality } => {
                write!(
                    f,
                    "layer quality {quality} has no equivalent — imported at full quality"
                )
            }
            Self::StretchAsRetime { percent } => {
                write!(
                    f,
                    "time stretch {percent}% imported as the equivalent Retime"
                )
            }
            Self::FlowEngineDiffers => write!(
                f,
                "Pixel Motion imported as flow interpolation — a different flow engine, so \
                 in-betweens differ"
            ),
            Self::ParentMissing { index } => {
                write!(
                    f,
                    "the parent layer {index} is not in this composition — imported unparented"
                )
            }
            Self::MatteTargetMissing { index } => {
                write!(
                    f,
                    "the matte layer {index} is not in this composition — imported with no matte"
                )
            }
            Self::BlendModeUnavailable { ae_mode } => {
                write!(
                    f,
                    "blend mode {ae_mode} has no equivalent — imported as Normal"
                )
            }
            Self::BlendModeClassic { ae_mode } => {
                write!(f, "{ae_mode} imported as its modern counterpart")
            }
            Self::ShapeContentsNotMapped => write!(
                f,
                "shape contents are not imported yet — the layer keeps its place and draws nothing"
            ),
            Self::TextStylingNotMapped => {
                write!(
                    f,
                    "the words, size and fill colour imported; the rest of the styling did not"
                )
            }
            Self::LightKindApproximated { ae_kind } => {
                write!(
                    f,
                    "a {ae_kind} light has no equivalent — imported as the nearest kind"
                )
            }
            Self::SpatialTangentsFlattened => write!(
                f,
                "the motion path's spatial tangents are not carried — each axis animates on its own"
            ),
            Self::ExpressionCarried => {
                write!(
                    f,
                    "the expression imported as source text and drives the property"
                )
            }
            Self::ExpressionDisabledCarried => {
                write!(
                    f,
                    "a switched-off expression — its text is kept, and it drives nothing"
                )
            }
            Self::PropertyUnreadable { match_name } => write!(
                f,
                "After Effects could not read {match_name} itself, so there was nothing to import"
            ),
            Self::MaskModeUnavailable { ae_mode } => {
                write!(f, "mask mode {ae_mode} is not built yet — imported as Add")
            }
            Self::MaskFeatherAxesDiffer { x, y } => write!(
                f,
                "feather {x} × {y} has one width in Lumit — imported at their average"
            ),
            Self::MaskRotoBezierFlattened => write!(
                f,
                "a RotoBezier mask's curves are computed by After Effects — imported as its polygon"
            ),
            Self::EffectPlaceholder { match_name } => write!(
                f,
                "{match_name} imported as a placeholder — every parameter is kept and it renders \
                 nothing"
            ),
            Self::EffectParamNotCarried { effect, param } => write!(
                f,
                "{effect}'s {param} has no equivalent — the effect imported without it"
            ),
            Self::EffectParamApproximated {
                effect,
                param,
                imported_as,
            } => write!(f, "{effect}'s {param} imported as {imported_as}"),
            Self::EffectDiffers { effect, detail } => write!(f, "{effect} imported, and {detail}"),
            Self::EffectSpeedAsKeyframes { effect, param } => write!(
                f,
                "{effect}'s {param} read the clock — imported as keyframes running at the same \
                 rate"
            ),
            Self::EffectSuggestion {
                match_name,
                instead,
            } => write!(
                f,
                "{match_name} imported as a placeholder — {instead} does this job in Lumit"
            ),
            Self::EffectParamRebased { effect, param } => write!(
                f,
                "{effect}'s {param} was measured in After Effects' units — imported as the same \
                 length in Lumit's"
            ),
        }
    }
}

/// One line of the report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportRow {
    pub path: ItemPath,
    pub outcome: Outcome,
    pub reason: Reason,
}

impl std::fmt::Display for ReportRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path, self.reason)
    }
}

/// Everything the import has to say, plus what it counted (docs/11 §9).
///
/// `imported` counts the things that came across whole and therefore raised no
/// row; the other three counts are derived from the rows, so a summary line and
/// the list beneath it can never disagree.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ImportReport {
    /// How many items, layers, masks, keyed properties and effects arrived
    /// without a single adjustment.
    pub imported: usize,
    pub rows: Vec<ReportRow>,
}

/// The four counts a summary line shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Summary {
    pub imported: usize,
    pub adjusted: usize,
    pub placeholders: usize,
    pub skipped: usize,
}

impl std::fmt::Display for Summary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} items imported · {} adjusted · {} placeholders · {} skipped",
            self.imported, self.adjusted, self.placeholders, self.skipped
        )
    }
}

impl ImportReport {
    /// Note something that came across whole.
    pub fn imported(&mut self) {
        self.imported = self.imported.saturating_add(1);
    }

    /// Add a row. The one way rows are made, so every row has a path and a
    /// typed reason.
    pub fn row(&mut self, path: ItemPath, outcome: Outcome, reason: Reason) {
        self.rows.push(ReportRow {
            path,
            outcome,
            reason,
        });
    }

    /// The counts behind docs/11 §9's summary line.
    #[must_use]
    pub fn summary(&self) -> Summary {
        let count = |wanted: Outcome| self.rows.iter().filter(|r| r.outcome == wanted).count();
        Summary {
            imported: self.imported,
            adjusted: count(Outcome::Adjusted),
            placeholders: count(Outcome::Placeholder),
            skipped: count(Outcome::Skipped),
        }
    }

    /// Every row of one grade — the report panel's filter.
    #[must_use]
    pub fn of(&self, outcome: Outcome) -> Vec<&ReportRow> {
        self.rows.iter().filter(|r| r.outcome == outcome).collect()
    }

    /// Whether any row carries this reason. Mostly for tests and for the panel's
    /// "are there any expressions to work through?" question.
    #[must_use]
    pub fn has(&self, reason: &Reason) -> bool {
        self.rows.iter().any(|r| &r.reason == reason)
    }
}
