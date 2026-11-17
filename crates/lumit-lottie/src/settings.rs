//! How a vector export is asked for: the format, how text and pictures go
//! out, the frame rate, the span and the players the check judges against.
//!
//! Runs wherever the caller runs; nothing here does any work.

use lumit_core::time::FrameRate;

use crate::check::Players;

/// The file a vector export writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    /// A Lottie `.json`.
    #[default]
    Lottie,
    /// A `.lottie`: the same animation zipped with its pictures (dotLottie v2).
    DotLottie,
    /// An `.svg` that animates by itself in a browser.
    Svg,
}

impl Format {
    /// The extension the file takes, without the dot.
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Format::Lottie => "json",
            Format::DotLottie => "lottie",
            Format::Svg => "svg",
        }
    }

    /// The format by its key, as the bridge and the command line name it.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Format> {
        match key {
            "lottie" | "json" => Some(Format::Lottie),
            "dotlottie" | "lottie_zip" => Some(Format::DotLottie),
            "svg" => Some(Format::Svg),
            _ => None,
        }
    }

    /// The key [`Self::from_key`] reads.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Format::Lottie => "lottie",
            Format::DotLottie => "dotlottie",
            Format::Svg => "svg",
        }
    }
}

/// How a Text layer goes out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextMode {
    /// The letters' outlines, drawn the same by every player. An SVG is always
    /// written this way.
    #[default]
    Shapes,
    /// A Lottie text layer naming its font, which a player draws only with the
    /// font installed.
    Text,
}

/// Where the pictures of still footage go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageMode {
    /// Inside the file, as data. A `.lottie` always carries them in its zip.
    #[default]
    Embed,
    /// Written beside the file and named from it.
    Beside,
}

/// Which stretch of the comp goes out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Span {
    /// The work area when the comp has one, else the whole comp.
    #[default]
    WorkArea,
    /// The whole comp.
    Whole,
}

/// Everything a vector export is asked for.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub format: Format,
    pub text: TextMode,
    pub images: ImageMode,
    /// `None` exports at the comp's own rate.
    pub frame_rate: Option<FrameRate>,
    pub span: Span,
    /// Smallest file: names left out and numbers rounded to what can be seen.
    pub minify: bool,
    /// The players the check judges against. Never changes the bytes written.
    pub players: Players,
    /// An SVG plays on repeat, and a `.lottie` asks its player to loop.
    pub loop_playback: bool,
    /// The file's own name without its extension, which names the pictures
    /// written beside it (`stem_1.png`).
    pub file_stem: String,
    /// How pictures are compressed: `None` keeps them lossless (PNG), and
    /// `Some(q)` writes an opaque picture as a JPEG at quality `q`, 1 to 100.
    /// A picture with any see-through pixel stays PNG either way.
    pub image_quality: Option<u8>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            format: Format::Lottie,
            text: TextMode::Shapes,
            images: ImageMode::Embed,
            frame_rate: None,
            span: Span::WorkArea,
            minify: false,
            players: Players::ALL,
            loop_playback: true,
            file_stem: String::from("animation"),
            image_quality: None,
        }
    }
}
