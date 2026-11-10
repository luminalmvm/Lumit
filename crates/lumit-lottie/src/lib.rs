//! Vector export: a comp as a Lottie, a dotLottie or an animated SVG, the
//! check of what won't come across, and Lottie import.
//!
//! # In plain terms
//!
//! Interface and web motion is shipped as files a player draws, not as
//! video: a Lottie (`.json`) for apps and pages, a dotLottie (`.lottie`, the
//! same zipped with its pictures), or an SVG that animates by itself. Each is
//! a comp written as shapes and keys, which is what most of a motion design
//! already is in Lumit, so this crate writes them from the document directly:
//! paths stay paths, eased keys stay the same curves ([`keys`]), and only
//! what has no equal in the file is baked into keys ([`fit`]) or left out.
//!
//! Nothing here stops an export for what it can't carry. The [`check`] says,
//! layer by layer, what was left out, what only some players draw and what
//! was written differently, and the export window lists it before anything
//! is written.
//!
//! Import reads a Lottie back into a comp, with a report of what didn't come
//! across ([`import`]).
//!
//! # Threads
//!
//! Everything runs on the caller's thread, which is a bridge worker or the
//! command line's, never the UI thread. Long work checks [`Host::cancelled`]
//! between layers and between sampled frames.

use std::sync::Arc;

use lumit_core::model::Document;
use uuid::Uuid;

pub mod check;
pub mod colour;
pub mod fit;
pub mod import;
pub mod keys;
pub mod lottie;
mod picture;
pub mod settings;
pub mod svg;
pub mod time;
pub mod walk;

/// A finished export, in memory: what the window previews and what
/// [`write`](Exported::bytes) puts on disk.
#[derive(Debug, Clone, PartialEq)]
pub struct Exported {
    /// The file: Lottie JSON text, a `.lottie` zip, or SVG text.
    pub bytes: Vec<u8>,
    /// Pictures written beside the file (images set to Beside), by file name.
    pub beside: Vec<(String, Vec<u8>)>,
    /// What won't come across, left out first.
    pub issues: Vec<check::Issue>,
    /// How many frames a player shows.
    pub frames: u32,
    pub fps: f64,
    pub width: u32,
    pub height: u32,
}

/// A still picture as the Viewer draws it: display sRGB, eight bits, straight
/// alpha, at its own size.
#[derive(Debug, Clone, PartialEq)]
pub struct Still {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// What a footage item is to a vector export.
#[derive(Debug, Clone, PartialEq)]
pub enum Picture {
    /// A still, ready to embed.
    Still(Still),
    /// A moving picture, which no player draws.
    Video,
    /// The file can't be read.
    Missing,
}

/// What the caller supplies that this crate can't do itself: decoding
/// footage, the project's colour, and being told to stop.
pub trait Host {
    /// A footage item's picture, through Lumit's own decode and interpretation.
    fn picture(&self, item: Uuid) -> Picture;
    /// Working-space linear RGB as display sRGB, encoded, 0 to 1.
    fn display(&self, linear: [f64; 3]) -> [f64; 3] {
        colour::rec709_display(linear)
    }
    /// True once the export is no longer wanted.
    fn cancelled(&self) -> bool {
        false
    }
    /// How far through it is, 0 to 1.
    fn progress(&self, _done: f32) {}
}

/// Why an export or an import could not finish.
#[derive(Debug, thiserror::Error)]
pub enum LottieError {
    #[error("There is no composition to export.")]
    NoComp,
    #[error("A node graph composition can't be exported as a Lottie or SVG.")]
    NodeGraph,
    #[error("The export was stopped.")]
    Cancelled,
    #[error("The file couldn't be packed: {0}")]
    Zip(String),
    #[error("The file couldn't be written: {0}")]
    Json(String),
    #[error("This file isn't a Lottie.")]
    NotLottie,
    #[error("This Lottie is too large to open: {0}")]
    TooLarge(String),
}

/// `comp` of `doc` as `settings` ask, in memory.
pub fn export(
    doc: &Arc<Document>,
    comp: Uuid,
    settings: &settings::Settings,
    host: &dyn Host,
) -> Result<Exported, LottieError> {
    let c = doc.comp(comp).ok_or(LottieError::NoComp)?;
    if c.graph.is_some() {
        return Err(LottieError::NodeGraph);
    }
    match settings.format {
        settings::Format::Lottie | settings::Format::DotLottie => {
            lottie::export(doc, c, settings, host)
        }
        settings::Format::Svg => svg::export(doc, c, settings, host),
    }
}
