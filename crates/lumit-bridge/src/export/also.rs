//! The files an export writes beside its own, as the seam names them: a
//! format key and a suffix. Where each one goes is the engine's rule.

use crate::api::export::BridgeExportAlso;

/// The dialogue's extra files as the engine's.
#[cfg(feature = "media")]
pub(super) fn to_engine(
    also: &[BridgeExportAlso],
) -> Result<Vec<lumit_render::export::outputs::AlsoWrite>, String> {
    also.iter()
        .map(|a| {
            Ok(lumit_render::export::outputs::AlsoWrite {
                format: super::export_format(&a.codec)?,
                suffix: a.suffix.clone(),
            })
        })
        .collect()
}

/// The engine's extra files as the dialogue's.
#[cfg(feature = "media")]
pub(super) fn from_engine(
    also: &[lumit_render::export::outputs::AlsoWrite],
) -> Vec<BridgeExportAlso> {
    also.iter()
        .map(|a| BridgeExportAlso {
            codec: super::format_key(a.format).to_owned(),
            suffix: a.suffix.clone(),
        })
        .collect()
}

/// Where each extra file of an export to `out_path` is written, with its
/// format key. A key this build cannot write has no file.
pub(crate) fn paths(out_path: &str, also: &[BridgeExportAlso]) -> Vec<(String, String)> {
    #[cfg(feature = "media")]
    {
        also.iter()
            .filter_map(|a| {
                let extra = lumit_render::export::outputs::AlsoWrite {
                    format: super::export_format(&a.codec).ok()?,
                    suffix: a.suffix.clone(),
                };
                let path =
                    lumit_render::export::outputs::path_for(std::path::Path::new(out_path), &extra);
                Some((path.to_string_lossy().into_owned(), a.codec.clone()))
            })
            .collect()
    }
    #[cfg(not(feature = "media"))]
    {
        let _ = (out_path, also);
        Vec::new()
    }
}
