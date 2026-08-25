//! Every user-facing word the effects schema can hand to the interface, as one
//! walk of the catalogue — so the translation gate reads the engine's own list
//! instead of scraping source text (K-303).
//!
//! # In plain terms
//!
//! Effect names, parameter names, dropdown options, group headings and the
//! Add-effect menu's categories are written in Rust and cross the bridge as
//! plain English. The frontend looks each one up in its translation table by
//! that English text, and a test fails when the engine can send a word the
//! table has no entry for. That test used to *scrape the source code* for
//! quoted strings — which went blind the day the derive macro started
//! generating labels that never appear as literals anywhere. The engine is the
//! only thing that sees the generated labels, so the engine writes the list:
//! [`user_facing_labels`] walks the catalogue, a fixture file in this crate
//! carries the result, one Rust test keeps the fixture honest, and the Dart
//! gate reads the fixture.

use std::collections::BTreeSet;

use super::schema::{FxCategory, ParamKind};

/// Every label the effects schema can put on screen: effect names, parameter
/// labels, Choice options, non-empty group headings, category headings.
/// Sorted and deduplicated, which is what makes the fixture diff stable.
#[must_use]
pub fn user_facing_labels() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for schema in super::BUILTINS {
        out.insert(schema.label.to_owned());
        for param in schema.params {
            out.insert(param.label.to_owned());
            // The Lens flare's lens picker lists the lens library by name —
            // manufacturer and model, proper nouns in every language. They are
            // deliberately not translated: the table's fallback shows an
            // unlisted label as it arrived, which for a Leica Summilux is the
            // only correct rendering. Every other Choice list is real UI copy.
            if schema.match_name == "lens_flare" && param.id == "lens_model" {
                continue;
            }
            if let ParamKind::Choice { options, .. } = param.kind {
                for option in options {
                    out.insert((*option).to_owned());
                }
            }
        }
        for group in schema.groups {
            // An empty label renders headerless — nothing reaches the screen.
            if !group.label.is_empty() {
                out.insert(group.label.to_owned());
            }
        }
    }
    for category in FxCategory::ALL {
        out.insert(category.label().to_owned());
    }
    // The graph canvas's own words (K-471): the two derived nodes, the ports
    // no schema declares, and every driver's declared output. The bridge draws
    // a socket from each, so each is a word the engine can send.
    out.insert(crate::graph::SOURCE_LABEL.to_owned());
    out.insert(crate::graph::OUT_LABEL.to_owned());
    for port in crate::graph::DERIVED_PORTS {
        out.insert(port.label.to_owned());
    }
    for def in super::BUILTIN_DEFS.iter() {
        for port in def.signature().outputs() {
            out.insert(port.label.to_owned());
        }
    }
    // The blend modes cross the same seam: `list_blend_modes` sends each
    // mode's display name for the Timeline's dropdown, and a name with no
    // table entry ships in English inside a translated application.
    for mode in crate::model::BlendMode::ALL {
        out.insert(mode.name().to_owned());
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Where the fixture lives: crate root, so the Flutter l10n gate can read
    /// it as `../crates/lumit-core/fx-labels.txt` from `flutter_ui/`.
    fn fixture_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fx-labels.txt")
    }

    fn rendered() -> String {
        let mut s = String::new();
        s.push_str(
            "# Every user-facing label the effects schema can send (K-303).\n\
             # Generated - do not edit. To refresh after a schema change:\n\
             #   cargo test -p lumit-core regenerate_fx_label_fixture -- --ignored\n",
        );
        for label in user_facing_labels() {
            s.push_str(&label);
            s.push('\n');
        }
        s
    }

    /// The fixture is the catalogue's own list. A schema change that adds or
    /// renames a label fails here until the fixture is regenerated — and the
    /// Dart gate (`flutter_ui/test/l10n/engine_labels_test.dart`) then fails
    /// until the translation table has the new word. Two tests, one chain,
    /// no source scraping anywhere.
    #[test]
    fn the_fx_label_fixture_matches_the_catalogue() {
        let want = rendered();
        let got = std::fs::read_to_string(fixture_path()).unwrap_or_default();
        assert_eq!(
            got, want,
            "fx-labels.txt is stale. Regenerate it:\n  cargo test -p lumit-core \
             regenerate_fx_label_fixture -- --ignored"
        );
    }

    /// A walk that suddenly finds almost nothing is a broken walk, not a small
    /// catalogue — the guard the old scrape had, kept for the same reason.
    #[test]
    fn the_catalogue_has_the_labels_a_catalogue_of_91_should() {
        assert!(
            user_facing_labels().len() > 600,
            "the catalogue declares far more labels than this - the walk has lost \
             a source of them"
        );
    }

    #[test]
    #[ignore = "writes the fixture; run after a schema change"]
    fn regenerate_fx_label_fixture() {
        std::fs::write(fixture_path(), rendered()).expect("write fx-labels.txt");
    }
}
