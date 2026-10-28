//! The built-in flares, compiled in.

use super::design::{Design, Element, Kind};

/// Each built-in flare's group and its design as JSON, in the order the
/// window shows them. The first is what a fresh Sprite flare draws.
pub const BUILT_IN: &[(&str, &str)] = &[
    ("Natural", include_str!("presets/natural-daylight.flare")),
    ("Natural", include_str!("presets/natural-golden-hour.flare")),
    (
        "Natural",
        include_str!("presets/natural-cold-morning.flare"),
    ),
    (
        "Natural",
        include_str!("presets/natural-through-leaves.flare"),
    ),
    ("Natural", include_str!("presets/natural-window.flare")),
    ("Natural", include_str!("presets/natural-candle.flare")),
    ("Natural", include_str!("presets/natural-dusty-lens.flare")),
    (
        "Natural",
        include_str!("presets/natural-rain-on-glass.flare"),
    ),
    ("Natural", include_str!("presets/natural-frost.flare")),
    ("Cinema", include_str!("presets/cinema-anamorphic.flare")),
    ("Cinema", include_str!("presets/cinema-amber-streak.flare")),
    ("Cinema", include_str!("presets/cinema-night-street.flare")),
    ("Cinema", include_str!("presets/cinema-vintage.flare")),
    ("Cinema", include_str!("presets/cinema-hairline.flare")),
    ("Cinema", include_str!("presets/cinema-wide-screen.flare")),
    (
        "Cinema",
        include_str!("presets/cinema-scratched-lens.flare"),
    ),
    ("Cinema", include_str!("presets/cinema-smudged-lens.flare")),
    ("Cinema", include_str!("presets/cinema-old-prime.flare")),
    ("Sci-fi", include_str!("presets/scifi-reactor.flare")),
    ("Sci-fi", include_str!("presets/scifi-hologram.flare")),
    ("Sci-fi", include_str!("presets/scifi-warp.flare")),
    ("Sci-fi", include_str!("presets/scifi-beacon.flare")),
    ("Sci-fi", include_str!("presets/scifi-plasma.flare")),
    ("Sci-fi", include_str!("presets/scifi-ion.flare")),
    ("Sci-fi", include_str!("presets/scifi-portal.flare")),
    ("Sci-fi", include_str!("presets/scifi-signal.flare")),
    ("Stage", include_str!("presets/stage-spotlight.flare")),
    ("Stage", include_str!("presets/stage-concert.flare")),
    ("Stage", include_str!("presets/stage-par-can.flare")),
    ("Stage", include_str!("presets/stage-laser.flare")),
    ("Stage", include_str!("presets/stage-flashbulb.flare")),
    ("Stage", include_str!("presets/stage-smoky-stage.flare")),
    ("Stage", include_str!("presets/stage-disco.flare")),
    ("Stage", include_str!("presets/stage-neon.flare")),
    ("Sun", include_str!("presets/sun-midday.flare")),
    ("Sun", include_str!("presets/sun-low-sun.flare")),
    ("Sun", include_str!("presets/sun-sunset.flare")),
    ("Sun", include_str!("presets/sun-winter-sun.flare")),
    ("Sun", include_str!("presets/sun-haze.flare")),
    ("Sun", include_str!("presets/sun-harsh-sun.flare")),
    ("Sun", include_str!("presets/sun-dawn.flare")),
    ("Sun", include_str!("presets/sun-ice-halo.flare")),
    ("Subtle", include_str!("presets/subtle-glint.flare")),
    ("Subtle", include_str!("presets/subtle-sparkle.flare")),
    ("Subtle", include_str!("presets/subtle-soft-glow.flare")),
    ("Subtle", include_str!("presets/subtle-whisper.flare")),
    ("Subtle", include_str!("presets/subtle-bokeh.flare")),
    ("Subtle", include_str!("presets/subtle-pinpoint.flare")),
    ("Subtle", include_str!("presets/subtle-streetlight.flare")),
    ("Subtle", include_str!("presets/subtle-wet-glass.flare")),
];

/// The groups the built-in flares are filed under.
pub const GROUPS: &[&str] = &["Natural", "Cinema", "Sci-fi", "Stage", "Sun", "Subtle"];

/// What a fresh Sprite flare draws: the first built-in flare, or a glow with
/// a train of ghosts when there are none.
pub fn default_design() -> Design {
    if let Some(d) = BUILT_IN.first().and_then(|(_, json)| Design::parse(json)) {
        return d;
    }
    Design {
        lumit_flare: super::design::FORMAT,
        name: String::new(),
        elements: vec![Element::new(Kind::Glow), Element::new(Kind::Ghosts)],
    }
}
