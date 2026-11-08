//! Style packages across the seam: a style's folder zipped into one
//! `.lumstyle` file. `lumit-extensions` judges and unpacks. The frontend
//! reads and uses the style once it is in the styles folder.

use std::path::Path;

use flutter_rust_bridge::frb;
use lumit_extensions::style::StyleError;

/// How taking in or making a style package went.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeStyleOutcome {
    /// Done. The style's name, which is its folder's.
    Ready { name: String },
    /// Not a zip, or no single style file in it.
    NotAStyle,
    /// Over the caps an extension package is held to.
    TooLarge,
    /// A link, or a file that would land outside the style's folder.
    Unsafe,
    /// The files would not write.
    Failed,
}

#[frb(ignore)]
fn outcome(done: Result<String, StyleError>) -> BridgeStyleOutcome {
    match done {
        Ok(name) => BridgeStyleOutcome::Ready { name },
        Err(StyleError::NotAStyle) => BridgeStyleOutcome::NotAStyle,
        Err(StyleError::TooLarge) => BridgeStyleOutcome::TooLarge,
        Err(StyleError::Unsafe) => BridgeStyleOutcome::Unsafe,
        Err(StyleError::Io(_)) => BridgeStyleOutcome::Failed,
    }
}

/// Unpack the style package at `path` into a folder of its own in
/// `styles`, over a style of the same name.
///
/// Not sync: a package of pictures and fonts takes a moment to unpack.
#[must_use]
pub fn style_install_package(path: String, styles: String) -> BridgeStyleOutcome {
    outcome(
        lumit_extensions::style::install_style_package(Path::new(&path), Path::new(&styles))
            .map(|(name, _)| name),
    )
}

/// The name of the style in the package at `path`, judged as an install
/// would, with nothing unpacked.
#[frb(sync)]
#[must_use]
pub fn style_read_package(path: String) -> BridgeStyleOutcome {
    outcome(lumit_extensions::style::read_style_package(Path::new(
        &path,
    )))
}

/// Make a package at `out` of the style file at `style`: the file alone,
/// or with `whole_folder` the folder it sits in.
///
/// Not sync: a folder of pictures takes a moment to pack.
#[must_use]
pub fn style_make_package(style: String, whole_folder: bool, out: String) -> BridgeStyleOutcome {
    outcome(lumit_extensions::style::pack_style(
        Path::new(&style),
        whole_folder,
        Path::new(&out),
    ))
}
