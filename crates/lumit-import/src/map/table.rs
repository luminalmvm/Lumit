//! The effect mapping table as a shipped data file
//! ([docs/11-AE-IMPORT.md](../../../../docs/11-AE-IMPORT.md) §5).
//!
//! # In plain terms
//!
//! After Effects names each of its effects with a **match name** — `ADBE Twirl`
//! — that stays the same however the user renames the effect in their project.
//! The importer needs a list saying which Lumit effect each of those names
//! becomes, and that list is `ae-effect-map.toml`, sitting beside the
//! application rather than buried in the program.
//!
//! Why beside it: the names are Adobe's, not ours, and we get them wrong. Of
//! the sixty this table started with, twelve were wrong until a sitting with a
//! live After Effects corrected them — a wrong name is an effect that quietly
//! arrives as a placeholder instead of the real thing. A file anyone can open
//! and fix is a correction that ships as a download rather than a release.
//!
//! What the file does **not** hold is the arithmetic: which dial becomes which
//! dial, and what happens to the number on the way. That lives in
//! [`super::fx_colour`] and [`super::fx_distort`], because a change of base, an
//! option list that maps by position, and a control After Effects splits where
//! Lumit joins are all code — the table names which of those conversions a row
//! uses, and the code holds it. A row naming a conversion this build does not
//! have is simply not claimed, so an edited file can never make the importer do
//! something it has no code for; the effect takes the placeholder road, which is
//! where an unknown effect goes anyway.
//!
//! Loading is once per process: an edited copy next to the executable wins, and
//! anything wrong with it — missing, unreadable, not valid TOML — falls back to
//! the copy compiled into the application, so a mistyped override never leaves
//! the importer with no table at all.

use std::collections::HashMap;
use std::sync::OnceLock;

/// The table compiled into the application — the one every test reads, and the
/// fallback when no override is present or the override cannot be parsed.
const SHIPPED: &str = include_str!("../../ae-effect-map.toml");

/// The file an override is looked for under, beside the executable.
const OVERRIDE_FILE: &str = "ae-effect-map.toml";

/// One row: an After Effects effect and what becomes of it.
#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct Row {
    /// After Effects' match name — the key.
    pub ae: String,
    /// After Effects' own name for the effect, as the import report says it.
    pub name: String,
    /// The Lumit effect's match name (docs/08-EFFECTS.md). Absent on a
    /// deliberate-placeholder row.
    #[serde(default)]
    pub lumit: String,
    /// Which conversion in `fx_colour`/`fx_distort` carries the controls
    /// across. Absent on a deliberate-placeholder row.
    #[serde(default)]
    pub conversion: String,
    /// A deliberate placeholder (docs/11 §5): the report names what does the
    /// job in Lumit instead.
    #[serde(default)]
    pub suggest: Option<String>,
    /// The OFX plugin identifiers that are **this same effect** — the vendor's
    /// own OFX build of the plug-in the After Effects match name belongs to
    /// (docs/11 §5, K-655).
    ///
    /// If one of them is in the catalogue this session — the user has the
    /// plug-in installed — the effect maps straight to it rather than to
    /// Lumit's nearest likeness, because the plug-in *is* the effect. The rule
    /// is **equality with a discovered identifier**, never a resemblance
    /// between labels: two products with similar names are two products. A
    /// list rather than one string because a vendor renames its identifier
    /// between eras (GenArts to Boris FX) and both eras are the same effect.
    ///
    /// An identifier this file has wrong fails the safe way: nothing matches
    /// it, and the effect takes the `lumit`/`conversion` road below, exactly as
    /// on a machine without the plug-in.
    #[serde(default)]
    pub ofx: Vec<String>,
    /// The match name is the famous one rather than an audited one (K-414).
    /// Carried for the record; nothing in the import branches on it, because a
    /// name that turns out to be wrong claims nothing and the effect becomes a
    /// placeholder with every parameter kept.
    #[serde(default)]
    #[allow(dead_code)]
    pub pending_audit: bool,
}

/// The parsed file.
#[derive(Debug, Default, serde::Deserialize)]
struct File {
    #[serde(default)]
    effect: Vec<Row>,
}

/// The table, keyed by After Effects match name.
#[derive(Debug, Default)]
pub(crate) struct Table {
    rows: HashMap<String, Row>,
}

impl Table {
    /// The row for one After Effects match name, or `None` — which is what
    /// sends the effect down the placeholder road (docs/11 §6).
    pub(crate) fn row(&self, ae: &str) -> Option<&Row> {
        self.rows.get(ae)
    }

    /// Every row, for the tests that walk the whole table.
    #[cfg(test)]
    pub(crate) fn rows(&self) -> impl Iterator<Item = &Row> {
        self.rows.values()
    }

    /// Parse one file's text. `None` when it is not valid TOML or holds no
    /// rows at all — an empty table would silently turn every import into
    /// placeholders, which is worse than ignoring the file.
    fn parse(text: &str) -> Option<Table> {
        let file: File = toml::from_str(text).ok()?;
        if file.effect.is_empty() {
            return None;
        }
        Some(Table {
            rows: file.effect.into_iter().map(|r| (r.ae.clone(), r)).collect(),
        })
    }
}

/// The table this process imports through.
pub(crate) fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| {
        override_text()
            .as_deref()
            .and_then(Table::parse)
            // The shipped file is checked in and parsed by a test in this
            // module, so this default is unreachable rather than a silent
            // degradation — and it is a default rather than a panic because
            // engine crates do not panic (docs/14 §4).
            .or_else(|| Table::parse(SHIPPED))
            .unwrap_or_default()
    })
}

/// The edited copy beside the executable, if there is one.
fn override_text() -> Option<String> {
    let path = std::env::current_exe().ok()?.parent()?.join(OVERRIDE_FILE);
    std::fs::read_to_string(path).ok()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// **The shipped file parses and every row is whole** — the test that makes
    /// [`table`]'s fallback unreachable rather than a silent degradation.
    #[test]
    fn the_shipped_table_parses() {
        let table = Table::parse(SHIPPED).expect("the shipped table parses");
        for row in table.rows() {
            assert!(!row.name.is_empty(), "{} has no AE name", row.ae);
            // An OFX identifier is a reverse-domain name compared for equality
            // (K-655). A blank or bare word would either match nothing or, far
            // worse, look like it had been checked.
            for id in &row.ofx {
                assert!(
                    id.contains('.') && !id.starts_with("ofx:"),
                    "{}'s OFX identifier \"{id}\" is not a plug-in identifier",
                    row.ae
                );
            }
            match &row.suggest {
                Some(instead) => {
                    assert!(!instead.is_empty(), "{} suggests nothing", row.ae);
                    assert!(
                        row.lumit.is_empty() && row.conversion.is_empty(),
                        "{} is a placeholder row and must map to nothing",
                        row.ae
                    );
                }
                None => {
                    assert!(!row.lumit.is_empty(), "{} names no Lumit effect", row.ae);
                    assert!(
                        lumit_core::fx::instantiate(&row.lumit).is_some(),
                        "{} maps to \"{}\", which this build does not ship",
                        row.ae,
                        row.lumit
                    );
                    assert!(!row.conversion.is_empty(), "{} names no conversion", row.ae);
                }
            }
        }
    }

    /// **A broken override is ignored, not obeyed.** Anything that is not valid
    /// TOML, and anything valid that holds no rows, leaves the shipped table in
    /// place — the property the fallback exists for.
    #[test]
    fn a_broken_override_falls_back() {
        assert!(Table::parse("this is not toml {{{").is_none());
        assert!(Table::parse("version = 1\n").is_none());
        let one = Table::parse(
            "version = 1\n[[effect]]\nae = \"X\"\nname = \"X\"\nlumit = \"blur\"\n\
             conversion = \"gaussian_blur\"\n",
        )
        .expect("one row is a table");
        assert!(one.row("X").is_some());
        assert!(one.row("ADBE Gaussian Blur 2").is_none());
    }
}
