//! A dotLottie (v2): the animation and its pictures in one zip.
//!
//! `manifest.json` first, then `a/<id>.json`, then each picture under `i/`,
//! deflated, every entry dated 1980 so the same animation zips to the same
//! bytes. One animation, no themes and no state machines.
//!
//! Runs on the export's thread.

use std::io::{Cursor, Write};

use serde_json::json;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipWriter};

use crate::LottieError;

/// `json` and `pictures` (file name, bytes) as a `.lottie`, the animation
/// named from `stem`.
pub fn pack(
    stem: &str,
    json: &[u8],
    pictures: &[(String, Vec<u8>)],
) -> Result<Vec<u8>, LottieError> {
    let id = animation_id(stem);
    let manifest = json!({
        "version": "2",
        "generator": "Lumit",
        "animations": [{"id": id}],
    });
    let manifest = serde_json::to_vec(&manifest).map_err(|e| LottieError::Json(e.to_string()))?;
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(DateTime::default());
    let zip_err = |e: zip::result::ZipError| LottieError::Zip(e.to_string());
    let io_err = |e: std::io::Error| LottieError::Zip(e.to_string());
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("manifest.json", options).map_err(zip_err)?;
    zip.write_all(&manifest).map_err(io_err)?;
    zip.start_file(format!("a/{id}.json"), options)
        .map_err(zip_err)?;
    zip.write_all(json).map_err(io_err)?;
    for (name, bytes) in pictures {
        zip.start_file(format!("i/{name}"), options)
            .map_err(zip_err)?;
        zip.write_all(bytes).map_err(io_err)?;
    }
    Ok(zip.finish().map_err(zip_err)?.into_inner())
}

/// An animation id from a file's name: letters, digits, `-` and `_` only,
/// so it is a safe file name inside the zip on every system.
fn animation_id(stem: &str) -> String {
    let id: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if id.trim_matches('_').is_empty() {
        String::from("animation")
    } else {
        id
    }
}
