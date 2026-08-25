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
    /// The footage file was fine in After Effects and is not on *this*
    /// machine — the ordinary case for a bundle carried across from another
    /// one. The item imports offline with its interpretation intact, ready for
    /// the standard relink (docs/11 §2.5); import never waits for media.
    MediaNotFound,
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
    /// After Effects rides a layer's two audio channels apart; Lumit has one
    /// level, so the left channel is what arrives.
    AudioLevelsDiffer { left: f64, right: f64 },
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
    /// A record in the `.aep` itself that this build could not read, named by
    /// its chunk id. Only the direct route raises it (K-418, docs/11 §7: a
    /// parse failure on one chunk skips that chunk and continues, and the
    /// report lists what was skipped).
    ChunkUnreadable { chunk: String },

    // --- masks ---
    /// AE feathers a mask separately in x and y; Lumit has one width.
    MaskFeatherAxesDiffer { x: f64, y: f64 },
    /// A RotoBezier mask: AE computes its tangents rather than storing them,
    /// so the imported path is the polygon through its vertices.
    MaskRotoBezierFlattened,

    // --- effects ---
    /// No mapping for this match name yet, so the instance is inert and keeps
    /// its complete dump (docs/11 §6).
    EffectPlaceholder { match_name: String },
    /// Several parameters of one placeholder instance that After Effects
    /// itself could not read, counted rather than listed. A third-party
    /// effect's dump is mostly blobs — Particular alone refuses dozens — and
    /// one row per parameter buries every other row in the report. The
    /// parameters are all kept in the instance's `ae` namespace either way; a
    /// single refused parameter still names itself with
    /// [`Self::PropertyUnreadable`].
    EffectParamsUnreadable { count: usize },
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
    /// sides measure in different bases: After Effects' per cents of the layer
    /// or points in the frame against Lumit's px@comp and per cents of the
    /// frame (docs/08 §2.3). Nothing was approximated — the same length is
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
            Self::MediaNotFound => write!(
                f,
                "the file is not on this machine — imported offline, ready to relink"
            ),
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
            Self::AudioLevelsDiffer { left, right } => write!(
                f,
                "audio levels {left} dB left and {right} dB right have one level in \
                 Lumit — imported at the left"
            ),
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
            Self::ChunkUnreadable { chunk } => write!(
                f,
                "a record in the project file ({chunk}) could not be read and was skipped"
            ),
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
            Self::EffectParamsUnreadable { count } => write!(
                f,
                "{count} parameters could not be read — they are kept whole and import as nothing"
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

impl Reason {
    /// The stable id this reason crosses the bridge under.
    ///
    /// [`Display`] writes the sentence with `format!`, and a sentence built
    /// that way cannot be translated — the lookup a frontend does is by whole
    /// text (docs/17 §"Display text crosses the bridge in English"). So the
    /// *pieces* cross instead: this id says which sentence, [`Self::args`] says
    /// what goes in its blanks, and the frontend writes it in the reader's
    /// language.
    ///
    /// It is serde's own tag rather than a hand-written table, so a variant
    /// added here gets an id whether or not anybody remembers to give it one.
    ///
    /// [`Display`]: std::fmt::Display
    #[must_use]
    pub fn key(&self) -> String {
        self.as_object()
            .get("reason")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string()
    }

    /// The facts this reason carries, by field name — "ae_mode" → "Dissolve",
    /// "percent" → "50". Named rather than positional so the frontend's
    /// sentence may put them in whatever order its language wants.
    #[must_use]
    pub fn args(&self) -> std::collections::BTreeMap<String, String> {
        self.as_object()
            .into_iter()
            .filter(|(name, _)| name.as_str() != "reason")
            .map(|(name, value)| (name, plain(&value)))
            .collect()
    }

    fn as_object(&self) -> serde_json::Map<String, serde_json::Value> {
        match serde_json::to_value(self) {
            Ok(serde_json::Value::Object(map)) => map,
            // Unreachable for this enum, and not worth a panic if it ever
            // stopped being: a reason with no id reads as one the frontend has
            // no sentence for, which falls back to the English above.
            _ => serde_json::Map::new(),
        }
    }
}

/// One JSON scalar as the frontend would print it. Whole floats lose their
/// ".0" so a stretch reads "50%" and not "50.0%", matching what [`Display`]
/// writes on the engine's side of the same fact.
///
/// [`Display`]: std::fmt::Display
fn plain(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Number(number) => number.as_f64().map_or_else(
            || number.to_string(),
            |f| {
                if f.fract() == 0.0 {
                    format!("{f:.0}")
                } else {
                    format!("{f}")
                }
            },
        ),
        other => other.to_string(),
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

    /// Fold the `PropertyUnreadable` rows raised since `mark` — the length
    /// [`Self::rows`] had before the effect's parameters were walked — into
    /// one count row filed against the effect instance at `path`.
    ///
    /// A placeholder for a third-party effect keeps every parameter, and the
    /// ones After Effects itself refused are a row each: Particular's forty-one
    /// blobs, Sapphire's, Optical Flares' — thousands of rows in a real
    /// project, which is a report nobody reads. One row per instance says the
    /// same thing. Below two the rows are left alone, because a lone refused
    /// parameter can afford to name itself.
    pub fn fold_unreadable_since(&mut self, mark: usize, path: ItemPath) {
        let unreadable = |row: &ReportRow| matches!(row.reason, Reason::PropertyUnreadable { .. });
        let mut tail = self.rows.split_off(mark.min(self.rows.len()));
        let count = tail.iter().filter(|row| unreadable(row)).count();
        if count < 2 {
            self.rows.append(&mut tail);
            return;
        }
        tail.retain(|row| !unreadable(row));
        self.rows.append(&mut tail);
        self.row(
            path,
            Outcome::Skipped,
            Reason::EffectParamsUnreadable { count },
        );
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// **A reason crosses as an id plus its facts, not as a sentence.**
    ///
    /// The whole of K-303 for the import report in one assertion: what goes
    /// over the bridge is `blend_mode_unavailable` + `{ae_mode: "Dissolve"}`,
    /// and the English sentence stays behind as the fallback. A frontend that
    /// got only the sentence could not translate it, because its lookup is by
    /// whole text and the text is different for every blend mode.
    #[test]
    fn a_reason_crosses_as_a_stable_id_and_its_named_facts() {
        let reason = Reason::BlendModeUnavailable {
            ae_mode: "Dissolve".into(),
        };
        assert_eq!(reason.key(), "blend_mode_unavailable");
        assert_eq!(reason.args()["ae_mode"], "Dissolve");

        // A reason with nothing to say carries no facts, and still has an id.
        assert_eq!(Reason::MediaPlaceholder.key(), "media_placeholder");
        assert!(Reason::MediaPlaceholder.args().is_empty());
    }

    /// **A whole number arrives whole.**
    ///
    /// The stretch percentage and the guessed frame rate are `f64`, and JSON
    /// spells them "50.0" and "24.0" where the report's own English says "50%"
    /// and "24 fps". The panel must read the same way the engine does, so the
    /// trailing zero is dropped on the way across — and a genuinely fractional
    /// figure is left alone.
    #[test]
    fn a_whole_number_loses_its_trailing_zero_and_a_fraction_keeps_its_digits() {
        let stretch = Reason::StretchAsRetime { percent: 50.0 };
        assert_eq!(stretch.args()["percent"], "50");
        assert!(stretch.to_string().contains("50%"));

        let par = Reason::PixelAspectIgnored { par: 1.4587 };
        assert_eq!(par.args()["par"], "1.4587");

        // Bools cross as words, which is what the frontend branches on for the
        // three "preserve when nested" sentences.
        let nested = Reason::NestedPreserveIgnored {
            fps: true,
            resolution: false,
        };
        assert_eq!(nested.args()["fps"], "true");
        assert_eq!(nested.args()["resolution"], "false");
    }
}
