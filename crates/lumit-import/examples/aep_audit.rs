//! What one import actually made of a project, headlessly:
//!
//! ```text
//! cargo run -p lumit-import --example aep_audit -- "<path to .aep or .lum-bundle>"
//! ```
//!
//! In plain terms: the test suite proves the import against fixtures small
//! enough to reason about, and this is how a *real* project gets looked at —
//! the two-hundred-comp, forty-clip, third-party-plug-in kind that no fixture
//! resembles. It imports, resolves the media the way the application does
//! (docs/11 §2.5), and prints the four counts of docs/11 §9's summary with the
//! reasons underneath, so "the import is broken" can become a number.
//!
//! It changes nothing on disk, and it is deliberately not a test: it needs a
//! project nobody can commit.

use std::collections::BTreeMap;
use std::path::PathBuf;

use lumit_core::model::ProjectItem;
use lumit_import::{ItemPath, Outcome, Reason};

fn main() {
    let Some(arg) = std::env::args().nth(1) else {
        println!("usage: aep_audit <path to .aep, .lum-bundle folder or zip>");
        return;
    };
    let path = PathBuf::from(arg);
    let bundle = match lumit_import::open_ae(&path) {
        Ok(bundle) => bundle,
        Err(error) => {
            println!("cannot open {}: {error}", path.display());
            return;
        }
    };

    let (mut doc, mut report) = lumit_import::map_capture(&bundle.capture);
    lumit_import::note_skipped_chunks(&bundle, &mut report);

    // The items, and the two facts a Project panel row cannot do without.
    let (mut folders, mut solids, mut comps, mut layers) = (0, 0, 0, 0);
    let (mut footage, mut nameless, mut pathless) = (0, 0, 0);
    for item in &doc.items {
        match item {
            ProjectItem::Folder(_) => folders += 1,
            ProjectItem::Solid(_) => solids += 1,
            ProjectItem::Composition(comp) => {
                comps += 1;
                layers += comp.layers.len();
            }
            ProjectItem::Footage(f) => {
                footage += 1;
                nameless += usize::from(f.name.trim().is_empty());
                pathless += usize::from(f.media.absolute_path.is_empty());
            }
        }
    }
    println!(
        "items: {folders} folders, {footage} footage ({nameless} nameless, \
         {pathless} pathless), {solids} solids, {comps} comps, {layers} layers"
    );

    // Where the media resolves, exactly as an import does it: against the
    // folder the file sits in (docs/11 §2.5's re-rooting step).
    let root = path.parent().unwrap_or(&path).to_path_buf();
    let (moved, missing) = lumit_project::resolve_all_media(&mut doc, &root, &[]);
    println!(
        "media: {} of {footage} resolve ({moved} found somewhere new)",
        footage - missing.len()
    );
    for name in missing.iter().take(20) {
        println!("  missing: {name}");
    }

    // The report the user would be reading, with the rows the bridge adds for
    // media that is missing now (`api/import.rs`).
    let already: Vec<String> = report
        .rows
        .iter()
        .filter(|row| matches!(row.reason, Reason::MediaMissing { .. }))
        .filter_map(|row| row.path.comp.clone())
        .collect();
    for name in &missing {
        if !already.contains(name) {
            report.row(
                ItemPath::item(name),
                Outcome::Adjusted,
                Reason::MediaNotFound,
            );
        }
    }

    let summary = report.summary();
    println!(
        "report: {} imported, {} adjusted, {} placeholders, {} skipped ({} rows)",
        summary.imported,
        summary.adjusted,
        summary.placeholders,
        summary.skipped,
        report.rows.len()
    );

    let mut by_reason: BTreeMap<String, usize> = BTreeMap::new();
    for row in &report.rows {
        let args: Vec<String> = row.reason.args().into_values().collect();
        *by_reason
            .entry(format!("{} {}", row.reason.key(), args.join(" ")))
            .or_default() += 1;
    }
    let mut rows: Vec<(String, usize)> = by_reason.into_iter().collect();
    rows.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (reason, n) in rows.iter().take(40) {
        println!("  {n:>5}  {reason}");
    }
}
