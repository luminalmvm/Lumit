//! The refusal taxonomy (docs/impl/ocio.md §1, §7.6).
//!
//! In plain terms: this crate would rather say "I cannot do that, and here is
//! exactly what I cannot do" than quietly do something *close*. A wrong picture
//! that looks plausible is the one failure this design refuses to ship, so every
//! corner of a config we have not implemented becomes one of the variants below,
//! and every variant **names the thing** — the transform, the style, the colour
//! space, the file — so the sentence the project settings row shows is
//! actionable rather than a shrug.
//!
//! The messages are the calm voice of docs/15-DESIGN.md: British English,
//! sentence case, one sentence, no blame. Engine-sent sentences that reach the
//! frontend need their `app_en.arb` keys — that is WP4's job, not this
//! crate's.

use std::path::PathBuf;

/// Everything this crate can refuse or fail on. Never a panic (docs/14 §4).
#[derive(Debug, thiserror::Error)]
pub enum ColourError {
    // ---- Refusals: things a config may legitimately contain that Lumit's v1
    // ---- transform set does not implement. Each names what it refused.
    /// A transform type outside the implemented op set (`FixedFunctionTransform`,
    /// the grading family, `ExposureContrastTransform`, …).
    #[error("this config needs {name}, which Lumit does not support yet")]
    UnsupportedTransform { name: String },

    /// A `BuiltinTransform` style in neither implemented tier (docs/impl/ocio.md §4.1).
    #[error("this config needs the built-in transform {style}, which Lumit does not support yet")]
    UnsupportedBuiltin { style: String },

    /// A 3D LUT cannot be inverted honestly, so the space that asked for it is named.
    #[error(
        "the colour space {space} would have to invert a 3D look-up table, which Lumit refuses rather than approximate"
    )]
    Unsupported3dLutInverse { space: String },

    /// A 1D LUT that does not rise or fall consistently has no single inverse.
    #[error("the curve in {path} does not rise or fall consistently, so it cannot be inverted")]
    NonMonotoneCurve { path: String },

    /// A matrix with no inverse (determinant zero).
    #[error("a matrix in this transform has no inverse")]
    SingularMatrix,

    /// A LUT file extension this crate does not read.
    #[error("Lumit does not read look-up tables in {extension} files yet")]
    UnsupportedLutFormat { extension: String },

    /// `$VAR`-style context variables in a file path (deliberately out of v1).
    #[error(
        "this config uses context variables in the path {path}, which Lumit does not support yet"
    )]
    ContextVariable { path: String },

    /// A CLF/CTF process node outside the implemented set.
    #[error("this look-up table uses the {node} process node, which Lumit does not support yet")]
    UnsupportedClfNode { node: String },

    /// `rawHalfs`, `halfDomain`, the mirror/pass-through exponent styles, and
    /// integer bit depths on nodes whose maths CLF defines on normalised values.
    #[error("this look-up table uses {feature}, which Lumit does not support yet")]
    UnsupportedClfFeature { feature: String },

    /// `ocio_profile_version` above 2.
    #[error(
        "this config is written for OCIO profile version {version}, which Lumit does not read yet"
    )]
    UnsupportedConfigVersion { version: String },

    // ---- Faults: the config is supported in principle but does not hold up.
    /// A name the config never declares.
    #[error("this config has no colour space named {name}")]
    UnknownColourSpace { name: String },

    #[error("this config does not define the {name} role")]
    UnknownRole { name: String },

    #[error("this config has no display named {name}")]
    UnknownDisplay { name: String },

    #[error("the display {display} has no view named {view}")]
    UnknownView { display: String, view: String },

    #[error("this config has no look named {name}")]
    UnknownLook { name: String },

    /// A `search_path` walk that found nothing.
    #[error("the look-up table {name} was not found on this config's search path")]
    LutFileNotFound { name: String },

    #[error("{path} could not be read: {reason}")]
    FileRead { path: PathBuf, reason: String },

    /// A grammar fault in `config.ocio`, a `.spi*` file or a CLF file.
    #[error("{what} could not be read: {reason}")]
    Parse { what: String, reason: String },

    /// A table larger than the budget allows (docs/14 §5).
    #[error(
        "a look-up table in {what} is larger than Lumit allows ({size} points, limit {limit})"
    )]
    TableTooLarge {
        what: String,
        size: usize,
        limit: usize,
    },
}

impl ColourError {
    /// The stable id this refusal crosses the bridge under (docs/17 "Display
    /// text crosses the bridge in English").
    ///
    /// In plain terms: the sentences above are English, and every one of them
    /// has a name or a file path in the middle of it — so a whole-text lookup
    /// could never translate them. The frontend writes its own sentence from
    /// this id and [`Self::args`], and shows the English above only when it has
    /// no sentence for the id.
    ///
    /// The id is the variant's own name in snake case, which is what lets
    /// `engine_labels_test.dart` read this enum and fail on a variant with no
    /// sentence. The match is exhaustive on purpose: a new refusal is a compile
    /// error here rather than a row that ships untranslated.
    #[must_use]
    pub fn key(&self) -> &'static str {
        match self {
            ColourError::UnsupportedTransform { .. } => "unsupported_transform",
            ColourError::UnsupportedBuiltin { .. } => "unsupported_builtin",
            ColourError::Unsupported3dLutInverse { .. } => "unsupported3d_lut_inverse",
            ColourError::NonMonotoneCurve { .. } => "non_monotone_curve",
            ColourError::SingularMatrix => "singular_matrix",
            ColourError::UnsupportedLutFormat { .. } => "unsupported_lut_format",
            ColourError::ContextVariable { .. } => "context_variable",
            ColourError::UnsupportedClfNode { .. } => "unsupported_clf_node",
            ColourError::UnsupportedClfFeature { .. } => "unsupported_clf_feature",
            ColourError::UnsupportedConfigVersion { .. } => "unsupported_config_version",
            ColourError::UnknownColourSpace { .. } => "unknown_colour_space",
            ColourError::UnknownRole { .. } => "unknown_role",
            ColourError::UnknownDisplay { .. } => "unknown_display",
            ColourError::UnknownView { .. } => "unknown_view",
            ColourError::UnknownLook { .. } => "unknown_look",
            ColourError::LutFileNotFound { .. } => "lut_file_not_found",
            ColourError::FileRead { .. } => "file_read",
            ColourError::Parse { .. } => "parse",
            ColourError::TableTooLarge { .. } => "table_too_large",
        }
    }

    /// The facts this refusal names, by field name — `name` → `ACES_RedMod03`,
    /// `space` → `fancy`.
    ///
    /// Named rather than positional so a translation may put them in whatever
    /// order its language wants. **These are the config's own words** (a colour
    /// space, a display, a file path) and are never translated, exactly as a
    /// codec name is not.
    #[must_use]
    pub fn args(&self) -> Vec<(&'static str, String)> {
        match self {
            ColourError::UnsupportedTransform { name }
            | ColourError::UnknownColourSpace { name }
            | ColourError::UnknownRole { name }
            | ColourError::UnknownDisplay { name }
            | ColourError::UnknownLook { name }
            | ColourError::LutFileNotFound { name } => vec![("name", name.clone())],
            ColourError::UnsupportedBuiltin { style } => vec![("style", style.clone())],
            ColourError::Unsupported3dLutInverse { space } => vec![("space", space.clone())],
            ColourError::NonMonotoneCurve { path } | ColourError::ContextVariable { path } => {
                vec![("path", path.clone())]
            }
            ColourError::SingularMatrix => Vec::new(),
            ColourError::UnsupportedLutFormat { extension } => {
                vec![("extension", extension.clone())]
            }
            ColourError::UnsupportedClfNode { node } => vec![("node", node.clone())],
            ColourError::UnsupportedClfFeature { feature } => vec![("feature", feature.clone())],
            ColourError::UnsupportedConfigVersion { version } => {
                vec![("version", version.clone())]
            }
            ColourError::UnknownView { display, view } => {
                vec![("display", display.clone()), ("view", view.clone())]
            }
            ColourError::FileRead { path, reason } => vec![
                ("path", path.display().to_string()),
                ("reason", reason.clone()),
            ],
            ColourError::Parse { what, reason } => {
                vec![("what", what.clone()), ("reason", reason.clone())]
            }
            ColourError::TableTooLarge { what, size, limit } => vec![
                ("what", what.clone()),
                ("size", size.to_string()),
                ("limit", limit.to_string()),
            ],
        }
    }
}

/// The shorthand this crate returns everywhere.
pub type Result<T> = std::result::Result<T, ColourError>;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Two refusals must never share an id, and every fact a sentence prints
    /// has to arrive by name as well — otherwise the frontend's own wording
    /// would have a hole where the config's name should be.
    #[test]
    fn every_refusal_has_its_own_id_and_names_its_facts() {
        let all = [
            ColourError::UnsupportedTransform {
                name: "FixedFunctionTransform".into(),
            },
            ColourError::UnsupportedBuiltin {
                style: "APPLE_LOG_to_ACES2065-1".into(),
            },
            ColourError::Unsupported3dLutInverse {
                space: "fancy".into(),
            },
            ColourError::NonMonotoneCurve {
                path: "curve.spi1d".into(),
            },
            ColourError::SingularMatrix,
            ColourError::UnsupportedLutFormat {
                extension: ".csp".into(),
            },
            ColourError::ContextVariable {
                path: "$SHOT/lut.spi3d".into(),
            },
            ColourError::UnsupportedClfNode {
                node: "ACES".into(),
            },
            ColourError::UnsupportedClfFeature {
                feature: "halfDomain".into(),
            },
            ColourError::UnsupportedConfigVersion {
                version: "3".into(),
            },
            ColourError::UnknownColourSpace {
                name: "nope".into(),
            },
            ColourError::UnknownRole {
                name: "aces_interchange".into(),
            },
            ColourError::UnknownDisplay {
                name: "Rec1886".into(),
            },
            ColourError::UnknownView {
                display: "sRGB".into(),
                view: "Log".into(),
            },
            ColourError::UnknownLook {
                name: "grade".into(),
            },
            ColourError::LutFileNotFound {
                name: "lut.spi3d".into(),
            },
            ColourError::FileRead {
                path: "config.ocio".into(),
                reason: "no such file".into(),
            },
            ColourError::Parse {
                what: "config.ocio".into(),
                reason: "line 4".into(),
            },
            ColourError::TableTooLarge {
                what: "lut.spi3d".into(),
                size: 512,
                limit: 128,
            },
        ];

        let mut keys = std::collections::BTreeSet::new();
        for e in &all {
            assert!(
                keys.insert(e.key()),
                "two refusals share the id {}",
                e.key()
            );
            // Every value the sentence prints must also arrive by name, so the
            // frontend's own wording can place it.
            for (_, value) in e.args() {
                assert!(
                    e.to_string().contains(&value),
                    "{}: the argument {value} is not in the sentence",
                    e.key()
                );
            }
        }
        assert_eq!(keys.len(), all.len());
    }
}
