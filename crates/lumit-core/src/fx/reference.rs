//! The effects catalogue as machine-readable JSON — the manual's source of
//! truth for every effect's parameter table (K-303's sibling for docs).
//!
//! # In plain terms
//!
//! The manual at docs.lumitlab.com wants a page per effect, and each page wants
//! a table listing every parameter: its name, what kind of control it is, its
//! range, its default, what unit the number is in. All of that is already
//! written down once, in Rust, on the effect's own declaration — so nothing
//! should retype it into a documentation page, where it would quietly go stale
//! the first time a slider's range changed.
//!
//! So the engine writes it out. [`reference_json`] walks the same `BUILTINS`
//! list the Add-effect menu reads and renders it as JSON; a fixture file in this
//! crate carries the result; one Rust test keeps the fixture honest, exactly as
//! `fx-labels.txt` does for the translation gate; and a small script in
//! `web-docs/scripts/` turns the fixture into the tables on the effect pages.
//! The prose on those pages — what an effect is for, what each control does to
//! the picture — is hand-written and the script never touches it.

use serde::Serialize;

use super::params::Unit;
use super::schema::{EffectSchema, EnabledCond, FxCategory, ParamKind};

/// The whole catalogue, ready to serialise.
#[derive(Debug, Serialize)]
pub struct Reference {
    /// A note for anyone who opens the fixture wondering what wrote it.
    pub generated_by: &'static str,
    /// Every category, in menu order.
    pub categories: Vec<Category>,
    /// Every effect, in catalogue (menu) order.
    pub effects: Vec<Effect>,
}

/// One Add-effect menu category.
#[derive(Debug, Serialize)]
pub struct Category {
    /// URL-safe form of `label`, and the page's directory name.
    pub slug: String,
    pub label: &'static str,
}

/// One built-in effect.
#[derive(Debug, Serialize)]
pub struct Effect {
    /// URL-safe form of `label`, and the page's file name.
    pub slug: String,
    pub match_name: &'static str,
    pub label: &'static str,
    pub version: u32,
    pub category: &'static str,
    pub category_slug: String,
    pub params: Vec<Param>,
    pub groups: Vec<Group>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub enabled_when: Vec<EnabledWhen>,
    /// The Matte row's deeper meaning, for the effects that claim it inside
    /// their own maths (K-395). Absent for the great majority, whose matte is
    /// the generic strength dissolve — that sentence is the same for all of
    /// them, and the manual writes it once in the Effects overview rather than
    /// thirty-odd times in thirty-odd tables.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matte: Option<Matte>,
    /// The Matte row's role, spelled out: "strength", "own" or "none". The
    /// manual needs the third value: Set matte declares no universal matte
    /// (K-429) yet owns rows *called* Matte and Invert, and without the role
    /// a generator that finds a param named `matte` prints the strength
    /// sentence for an effect the dissolve never runs on.
    pub matte_role: &'static str,
}

/// What one effect's Matte row means, when it is not simply strength.
#[derive(Debug, Serialize)]
pub struct Matte {
    /// The parameter the layer reference is stored under — `matte` for most,
    /// `depth` for Depth of field (K-065 keeps the older id).
    pub param: &'static str,
    /// One sentence, from the effect's own declaration.
    pub meaning: &'static str,
}

/// One declared parameter, flattened so a table renderer needs no `match`.
#[derive(Debug, Serialize)]
pub struct Param {
    pub id: &'static str,
    pub label: &'static str,
    /// `float`, `slider`, `int`, `angle`, `choice`, `bool`, `colour`, `seed`,
    /// `file`, `layer`, `mask_path`, `curve`, `action`.
    pub kind: &'static str,
    /// The declared unit: `raw`, `pct_diag`, `px`, `degrees`, `seconds`.
    pub unit: &'static str,
    /// The declared default. A number for Float/Int/Angle, a bool, an RGBA
    /// array for Colour, the option *index* for Choice, absent for the kinds
    /// whose default is per-instance (Seed) or empty (File, Layer).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
    /// Slider ends for Float/Int; the per-channel edit range for Colour.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slider_min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slider_max: Option<f64>,
    /// Hard bounds, where the schema declares them. Either side may be absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hard_min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hard_max: Option<f64>,
    /// The dial's snapping increment, in degrees (Angle only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dial_step: Option<f64>,
    /// Choice option labels, in index order.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<&'static str>>,
    /// Option indices after which the dropdown draws a divider.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dividers_after: Option<Vec<u32>>,
    /// File dialog filter (File only): extensions, then the filter's name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_filter: Option<Vec<&'static str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_filter_name: Option<&'static str>,
    /// Whether a fresh Layer reference points at its own layer — or, for a
    /// `mask_path`, whether an unset row means the layer's first mask.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub self_default: Option<bool>,
}

/// One collapsible parameter group.
#[derive(Debug, Serialize)]
pub struct Group {
    /// Empty renders headerless — the rows appear in place with no twirl.
    pub label: &'static str,
    pub params: Vec<&'static str>,
    pub collapsed: bool,
    /// `(sibling Choice id, the option indices that show this group)`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visible_when: Option<VisibleWhen>,
    /// Shown only while the lens in play has at least this many elements.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visible_when_lens_elements: Option<u32>,
}

/// The condition on a [`Group`].
#[derive(Debug, Serialize)]
pub struct VisibleWhen {
    pub param: &'static str,
    pub values: Vec<u32>,
}

/// One "this row greys out while that control says so" rule.
#[derive(Debug, Serialize)]
pub struct EnabledWhen {
    /// The parameter that greys out.
    pub param: &'static str,
    /// The parameter whose value decides it.
    pub on: &'static str,
    /// `bool_is`, `choice_is`, `choice_is_not`, `layer_set`.
    pub cond: &'static str,
    /// The value `on` must hold, where the condition names one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
}

/// A label as a URL segment: lower case, runs of anything else become one
/// hyphen. "Blur & sharpen" becomes `blur-sharpen`.
#[must_use]
pub fn slug(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    for ch in label.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_owned()
}

const fn unit_name(unit: Unit) -> &'static str {
    match unit {
        Unit::Raw => "raw",
        Unit::PctDiag => "pct_diag",
        Unit::Px => "px",
        Unit::Degrees => "degrees",
        Unit::Seconds => "seconds",
    }
}

/// `serde_json::Value` for an `f64` that is finite; `None` otherwise, so an
/// infinity in a schema becomes an absent field rather than invalid JSON.
fn num(v: f64) -> Option<serde_json::Value> {
    serde_json::Number::from_f64(v).map(serde_json::Value::Number)
}

fn param(schema: &'static super::schema::ParamSchema) -> Param {
    let mut p = Param {
        id: schema.id,
        label: schema.label,
        kind: "",
        unit: unit_name(schema.unit),
        default: None,
        slider_min: None,
        slider_max: None,
        hard_min: None,
        hard_max: None,
        dial_step: None,
        options: None,
        dividers_after: None,
        file_filter: None,
        file_filter_name: None,
        self_default: None,
    };
    match schema.kind {
        ParamKind::Float {
            default,
            slider,
            hard,
        } => {
            p.kind = "float";
            p.default = num(default);
            p.slider_min = Some(slider.0);
            p.slider_max = Some(slider.1);
            p.hard_min = hard.0;
            p.hard_max = hard.1;
        }
        // A closed range prints its ends as both the slider and the hard
        // bounds, because that is what closed means (K-414): the manual's
        // table then says the same thing for a Slider as for a Float whose
        // two ranges happen to coincide, and only the kind differs.
        ParamKind::Slider { default, range } => {
            p.kind = "slider";
            p.default = num(default);
            p.slider_min = Some(range.0);
            p.slider_max = Some(range.1);
            p.hard_min = Some(range.0);
            p.hard_max = Some(range.1);
        }
        ParamKind::Int {
            default,
            slider,
            hard,
        } => {
            p.kind = "int";
            p.default = Some(default.into());
            #[allow(clippy::cast_precision_loss)]
            {
                p.slider_min = Some(slider.0 as f64);
                p.slider_max = Some(slider.1 as f64);
                p.hard_min = hard.0.map(|v| v as f64);
                p.hard_max = hard.1.map(|v| v as f64);
            }
        }
        ParamKind::Angle { default, dial_step } => {
            p.kind = "angle";
            p.default = num(default);
            p.dial_step = Some(dial_step);
        }
        ParamKind::Choice {
            options,
            default,
            dividers_after,
        } => {
            p.kind = "choice";
            p.default = Some(default.into());
            p.options = Some(options.to_vec());
            p.dividers_after = Some(dividers_after.to_vec());
        }
        ParamKind::Bool { default } => {
            p.kind = "bool";
            p.default = Some(default.into());
        }
        ParamKind::Colour { default, range } => {
            p.kind = "colour";
            p.default = Some(serde_json::Value::Array(
                default.iter().filter_map(|c| num(*c)).collect(),
            ));
            p.slider_min = Some(range.0);
            p.slider_max = Some(range.1);
        }
        ParamKind::Seed => p.kind = "seed",
        ParamKind::File {
            filter,
            filter_name,
        } => {
            p.kind = "file";
            p.file_filter = Some(filter.to_vec());
            p.file_filter_name = Some(filter_name);
        }
        ParamKind::Layer { self_default } => {
            p.kind = "layer";
            p.self_default = Some(self_default);
        }
        ParamKind::MaskPath { self_default } => {
            p.kind = "mask_path";
            p.self_default = Some(self_default);
        }
        // A curve's default is its shape, so it prints as the point list
        // itself (K-412) — the identity diagonal, for every built-in that
        // declares one.
        ParamKind::Curve => {
            p.kind = "curve";
            p.default = Some(serde_json::Value::Array(
                crate::fx::params::CURVE_IDENTITY
                    .iter()
                    .map(|xy| {
                        serde_json::Value::Array(
                            xy.iter().filter_map(|v| num(f64::from(*v))).collect(),
                        )
                    })
                    .collect(),
            ));
        }
        // A button (K-417): no default, no range, no options — the label and
        // the kind are the whole of what the manual's table can say about it.
        ParamKind::Action => p.kind = "action",
    }
    p
}

fn effect(schema: &'static EffectSchema) -> Effect {
    Effect {
        slug: slug(schema.label),
        match_name: schema.match_name,
        label: schema.label,
        version: schema.version,
        category: schema.category.label(),
        category_slug: slug(schema.category.label()),
        params: schema.params.iter().map(param).collect(),
        groups: schema
            .groups
            .iter()
            .map(|g| Group {
                label: g.label,
                params: g.params.to_vec(),
                collapsed: g.collapsed,
                visible_when: g.visible_when.map(|(param, values)| VisibleWhen {
                    param,
                    values: values.to_vec(),
                }),
                visible_when_lens_elements: g.visible_when_lens_elements,
            })
            .collect(),
        matte: match schema.matte {
            super::schema::MatteRole::Own { param, meaning } => Some(Matte { param, meaning }),
            _ => None,
        },
        matte_role: match schema.matte {
            super::schema::MatteRole::None => "none",
            super::schema::MatteRole::Strength => "strength",
            super::schema::MatteRole::Own { .. } => "own",
        },
        enabled_when: schema
            .enabled_when
            .iter()
            .map(|e| {
                let (cond, value) = match e.cond {
                    EnabledCond::BoolIs(v) => ("bool_is", Some(v.into())),
                    EnabledCond::ChoiceIs(v) => ("choice_is", Some(v.into())),
                    EnabledCond::ChoiceIsNot(v) => ("choice_is_not", Some(v.into())),
                    EnabledCond::LayerSet => ("layer_set", None),
                };
                EnabledWhen {
                    param: e.param,
                    on: e.on,
                    cond,
                    value,
                }
            })
            .collect(),
    }
}

/// The whole catalogue, in menu order, params in schema order.
#[must_use]
pub fn reference() -> Reference {
    Reference {
        generated_by: "cargo test -p lumit-core regenerate_fx_reference -- --ignored",
        categories: FxCategory::ALL
            .iter()
            .map(|c| Category {
                slug: slug(c.label()),
                label: c.label(),
            })
            .collect(),
        effects: super::BUILTINS.iter().map(effect).collect(),
    }
}

/// The fixture's exact bytes: pretty JSON, trailing newline.
///
/// Returns the serialiser's error rather than panicking, though the only way
/// this fails is a non-finite number reaching it, which `num` already filters.
///
/// # Errors
/// If the catalogue cannot be serialised to JSON.
pub fn reference_json() -> Result<String, serde_json::Error> {
    Ok(serde_json::to_string_pretty(&reference())? + "\n")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Where the fixture lives: crate root, beside `fx-labels.txt`, so the
    /// manual's generator reads it as
    /// `../crates/lumit-core/fx-reference.json` from `web-docs/`.
    fn fixture_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fx-reference.json")
    }

    /// The fixture is the catalogue's own description. A schema change that
    /// adds a parameter, moves a slider end or renames a label fails here until
    /// the fixture is regenerated — and the manual's tables then regenerate
    /// from it, so a page cannot describe a control the engine stopped having.
    #[test]
    fn the_fx_reference_fixture_matches_the_catalogue() {
        let want = reference_json().unwrap();
        let got = std::fs::read_to_string(fixture_path()).unwrap_or_default();
        assert_eq!(
            got.replace("\r\n", "\n"),
            want,
            "fx-reference.json is stale. Regenerate it:\n  cargo test -p lumit-core \
             regenerate_fx_reference -- --ignored"
        );
    }

    /// A walk that suddenly finds almost nothing is a broken walk, not a small
    /// catalogue — the same guard `fx-labels.txt` keeps.
    #[test]
    fn every_effect_and_category_is_described() {
        let r = reference();
        assert_eq!(r.categories.len(), FxCategory::ALL.len());
        assert_eq!(r.effects.len(), super::super::BUILTINS.len());
        assert!(r.effects.len() >= 91, "the catalogue has lost effects");
        for e in &r.effects {
            assert!(!e.slug.is_empty(), "{} has no slug", e.match_name);
            assert!(!e.params.is_empty(), "{} declares no parameters", e.label);
        }
    }

    /// The slug is a file name and a URL segment — two effects sharing one
    /// would silently overwrite a page.
    #[test]
    fn slugs_are_unique_within_a_category() {
        let r = reference();
        let mut seen = std::collections::BTreeSet::new();
        for e in &r.effects {
            assert!(
                seen.insert((e.category_slug.clone(), e.slug.clone())),
                "two effects share the page {}/{}",
                e.category_slug,
                e.slug
            );
        }
    }

    #[test]
    fn slugging_folds_punctuation_and_case() {
        assert_eq!(slug("Blur & sharpen"), "blur-sharpen");
        assert_eq!(slug("Gaussian blur"), "gaussian-blur");
        assert_eq!(slug("RGB split"), "rgb-split");
    }

    #[test]
    #[ignore = "writes the fixture; run after a schema change"]
    fn regenerate_fx_reference() {
        std::fs::write(fixture_path(), reference_json().unwrap()).expect("write fx-reference.json");
    }
}
