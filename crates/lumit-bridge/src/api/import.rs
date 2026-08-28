//! Importing an After Effects project (docs/11-AE-IMPORT.md, docs/impl/ae-import.md
//! §6 phase 3) — the one call the File menu makes, and the report it answers with.
//!
//! In plain terms: the user points Lumit at a folder the Lumit Bridge wrote
//! inside After Effects. `lumit-import` reads it and builds a whole new
//! document; this module makes that document the open project, exactly the way
//! opening a `.lum` does — the previous project's render worker and its GPU
//! device are let go, its media caches are cleared, and its change stream is
//! forgotten — and hands the frontend the list of everything that did not come
//! across untouched.
//!
//! **The report's sentences are not written here.** A reason crosses as a
//! stable id plus its facts (`blend_mode_unavailable` + `ae_mode: "Dissolve"`),
//! because a sentence built with `format!` cannot be translated: the frontend's
//! lookup is by whole text, and "blend mode Dissolve has no equivalent" is a
//! different whole text for every blend mode (K-303, docs/17 §"Display text
//! crosses the bridge in English"). `english` rides along as the fallback for a
//! reason the frontend has no sentence for yet, which is the same courtesy
//! `engine_labels.dart` extends to an effect label it has never seen.

use std::path::{Path, PathBuf};

use flutter_rust_bridge::frb;
use lumit_import::{ItemPath, Outcome, Reason, ReportRow};

use crate::api::{
    project::ProjectReference,
    state::{adopt, CallbackStream, LumitBridgeState},
    BridgeError,
};

/// The four grades of docs/11 §9, as the panel's filter sees them.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeImportOutcome {
    Imported,
    Adjusted,
    Placeholder,
    Skipped,
}

impl From<Outcome> for BridgeImportOutcome {
    fn from(outcome: Outcome) -> Self {
        match outcome {
            Outcome::Imported => Self::Imported,
            Outcome::Adjusted => Self::Adjusted,
            Outcome::Placeholder => Self::Placeholder,
            Outcome::Skipped => Self::Skipped,
        }
    }
}

/// One blank in a reason's sentence, by name: `ae_mode` → `Dissolve`,
/// `percent` → `50`. Named rather than positional so a translation may put
/// them in whatever order its language wants.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeImportArg {
    pub name: String,
    pub value: String,
}

/// One line of the report (docs/11 §9): where it happened, what grade the
/// outcome got, and why.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeImportRow {
    /// The item, as a reader would say it — "Main ▸ clip.mp4 ▸ Position". The
    /// user's own words (comp and layer names), so it is not translated.
    pub path: String,
    pub outcome: BridgeImportOutcome,
    /// The stable id of the sentence to write — see the module note.
    pub reason: String,
    pub args: Vec<BridgeImportArg>,
    /// The engine's own English, for a frontend with no sentence for `reason`.
    pub english: String,
}

impl From<&ReportRow> for BridgeImportRow {
    fn from(row: &ReportRow) -> Self {
        Self {
            path: row.path.to_string(),
            outcome: row.outcome.into(),
            reason: row.reason.key(),
            args: row
                .reason
                .args()
                .into_iter()
                .map(|(name, value)| BridgeImportArg { name, value })
                .collect(),
            english: row.reason.to_string(),
        }
    }
}

/// Everything the import has to say: docs/11 §9's four counts, and the rows
/// beneath them.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeImportReport {
    pub imported: u32,
    pub adjusted: u32,
    pub placeholders: u32,
    pub skipped: u32,
    pub rows: Vec<BridgeImportRow>,
}

/// The imported project and its report, together, because a caller that got one
/// without the other would have to ask twice for two halves of one answer.
#[frb(non_opaque)]
pub struct BridgeImportedProject {
    pub project: ProjectReference,
    pub report: BridgeImportReport,
}

impl LumitBridgeState {
    /// Import an After Effects project and make it the open one — either front
    /// door (K-418): an `.aep` read directly, or a Lumit Bridge bundle as a
    /// `.lum-bundle` folder or a zip of one. `lumit_import::open_ae` decides
    /// which by the bytes, so this is one call and one report whichever the
    /// user picked.
    ///
    /// `None` when what was picked is not either of those, or is one this build
    /// cannot read: the previous project stays loaded and the frontend shows
    /// its own notice, exactly as [`LumitBridgeState::open_project`] does for a
    /// `.lum` that will not open. Anything short of that is not a failure — an
    /// import **always completes** (docs/11 §9), and what could not be carried
    /// across is in the report rather than in an error.
    ///
    /// The project it leaves open has **no path**: an import is not a file, and
    /// the first save must ask where to put it.
    ///
    /// Deliberately **not** `#[frb(sync)]`, for the reason `open_project` is
    /// not: this parses a whole capture, builds a document from it and stats
    /// every media file it names, and on Dart's UI isolate that is the window
    /// frozen for as long as it takes.
    pub fn import_ae_bundle(
        path: &str,
        on_change_stream: Option<CallbackStream>,
    ) -> Result<Option<BridgeImportedProject>, BridgeError> {
        let path = PathBuf::from(path);
        let Ok(bundle) = lumit_import::open_ae(&path) else {
            return Ok(None);
        };
        let (doc, mut report) = lumit_import::map_capture(&bundle.capture);

        // What the direct parser had to skip is the report's to say (docs/11
        // §7); the engine decides which rows those are.
        lumit_import::note_skipped_chunks(&bundle, &mut report);

        // Footage resolves against the bundle's own folder (docs/11 §2.5's
        // re-rooting step); a `.lum-bundle` given as a zip, and an `.aep`,
        // re-root against the folder the file is in. v1 does paths only — the
        // collected `footage/` copy is a later phase.
        let media_root = if path.is_dir() {
            path.clone()
        } else {
            path.parent().unwrap_or_else(|| Path::new("")).to_path_buf()
        };

        // What After Effects itself flagged as missing already has a row; those
        // items are missing here too, and saying so twice would be noise.
        let already: Vec<String> = report
            .rows
            .iter()
            .filter(|row| matches!(row.reason, Reason::MediaMissing { .. }))
            .filter_map(|row| row.path.comp.clone())
            .collect();

        // The project is unsaved — an import is not a file (see above) — so the
        // media root is passed separately rather than derived from a path.
        // No progress stream: an import runs behind its own card, and the one
        // phased bar there is belongs to opening a `.lum` (K-628).
        let (project, missing) = adopt(doc, None, &media_root, on_change_stream, None)?;

        for name in missing {
            if already.contains(&name) {
                continue;
            }
            report.row(
                ItemPath::item(&name),
                Outcome::Adjusted,
                Reason::MediaNotFound,
            );
        }

        let summary = report.summary();
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        Ok(Some(BridgeImportedProject {
            project,
            report: BridgeImportReport {
                imported: count(summary.imported),
                adjusted: count(summary.adjusted),
                placeholders: count(summary.placeholders),
                skipped: count(summary.skipped),
                rows: report.rows.iter().map(BridgeImportRow::from).collect(),
            },
        }))
    }
}
