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
//! frontend need their `app_en.arb` keys (K-005) — that is WP4's job, not this
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

    /// A colour space that declares neither direction, so it cannot be resolved.
    #[error("the colour space {name} declares no transform in either direction")]
    NoTransform { name: String },

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

/// The shorthand this crate returns everywhere.
pub type Result<T> = std::result::Result<T, ColourError>;
