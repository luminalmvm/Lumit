//! An After Effects animation preset (`.ffx`), read directly.
//!
//! **In plain terms.** A preset is a slice of a project file. It is the same
//! container holding the same property records, cut down to the effects
//! somebody saved, each with its values, its keyframes and its expressions.
//! So there is very little to do here: find each effect and hand it to the
//! reader the project parser already uses ([`super::props`]).
//!
//! The layout, as far as this reads it:
//!
//! ```text
//! RIFX FaFX
//!   head
//!   LIST besc
//!     beso                    the comp the preset was saved from
//!     LIST tdsp, tdsn  ×N     where each saved property sat, and its name
//!     LIST tdsp               the "end of path" sentinel
//!     LIST sspc        ×N     the properties themselves, in the same order
//! ```
//!
//! A preset can also hold a layer's own transform, its masks or a text
//! animator. Those are named in the skipped rows and left for later; the
//! effects are what a preset is nearly always for.

use super::props::{self, Ctx};
use super::rifx::{open_form, u16_at, u32_at, Chunk};
use super::AepError;
use crate::capture::{Property, Unreadable};

/// The first name on the path of a property that is an effect.
const EFFECT_PARADE: &str = "ADBE Effect Parade";
/// The one name on the path that closes the list of paths.
const SENTINEL: &str = "ADBE End of path sentinel";

/// Fixed offsets inside the `beso` record.
mod beso {
    /// How many of the file's time units make a second.
    pub const TIMEBASE: usize = 12;
    pub const COMP_WIDTH: usize = 24;
    pub const COMP_HEIGHT: usize = 26;
    pub const LAYER_WIDTH: usize = 28;
    pub const LAYER_HEIGHT: usize = 30;
}

/// Everything one preset holds that Lumit reads.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Preset {
    /// The effects, top of the stack first, as the capture's own nodes.
    pub effects: Vec<Property>,
    /// What was passed over, ready to be report rows.
    pub skipped: Vec<Unreadable>,
    /// The size of the layer the preset was saved from, which an effect's
    /// points are measured against.
    pub size: (f64, f64),
}

/// Whether these bytes are an After Effects animation preset.
#[must_use]
pub fn is_preset(bytes: &[u8]) -> bool {
    bytes.get(..4) == Some(b"RIFX") && bytes.get(8..12) == Some(b"FaFX")
}

/// Parse a preset out of the bytes of an `.ffx`.
///
/// A pure function over a slice, as [`super::parse_capture`] is: nothing here
/// touches the filesystem or can panic on a malformed byte.
pub fn parse_preset(bytes: &[u8]) -> Result<Preset, AepError> {
    let description = open_form(bytes, b"FaFX")?
        .ok()
        .find(|chunk| chunk.is_list(b"besc"))
        .ok_or(AepError::NoItemTree)?;
    let inside: Vec<Chunk<'_>> = description.children().ok().collect();

    let header = inside
        .iter()
        .find(|chunk| chunk.id == *b"beso")
        .map(|chunk| chunk.body)
        .unwrap_or_default();
    let side = |offset: usize, fallback: f64| match u16_at(header, offset) {
        Some(n) if n > 0 => f64::from(n),
        _ => fallback,
    };
    let comp = (
        side(beso::COMP_WIDTH, 1920.0),
        side(beso::COMP_HEIGHT, 1080.0),
    );
    let layer = (
        side(beso::LAYER_WIDTH, comp.0),
        side(beso::LAYER_HEIGHT, comp.1),
    );
    let ctx = Ctx {
        params: None,
        timebase: f64::from(u32_at(header, beso::TIMEBASE).unwrap_or_default()),
        comp,
        layer,
        has_source: true,
        start: 0.0,
        in_effect: false,
        layers: None,
    };

    // The paths come first and the properties after, in the same order.
    let paths: Vec<Vec<String>> = inside
        .iter()
        .filter(|chunk| chunk.is_list(b"tdsp"))
        .map(path_of)
        .filter(|path| path.first().map(String::as_str) != Some(SENTINEL))
        .collect();
    let saved = inside
        .iter()
        .filter(|chunk| chunk.list_type.is_some() && !chunk.is_list(b"tdsp"));

    let mut preset = Preset {
        size: layer,
        ..Preset::default()
    };
    for (path, chunk) in paths.iter().zip(saved) {
        let match_name = path.last().map(String::as_str).unwrap_or_default();
        if path.first().map(String::as_str) == Some(EFFECT_PARADE) && chunk.is_list(b"sspc") {
            preset.effects.push(props::read_effect(
                match_name,
                chunk,
                ctx,
                &mut preset.skipped,
            ));
        } else {
            preset.skipped.push(Unreadable {
                path: Some(path.join(" ▸ ")),
                match_name: Some(match_name.to_string()),
                error: Some("only the effects in a preset are read".to_string()),
                ..Unreadable::default()
            });
        }
    }
    Ok(preset)
}

/// The match names along one saved property's path, outermost first.
fn path_of(tdsp: &Chunk<'_>) -> Vec<String> {
    tdsp.children()
        .ok()
        .filter(|chunk| chunk.is_list(b"tdsi"))
        .filter_map(|step| step.children().ok().find(|chunk| chunk.id == *b"tdmn"))
        .map(|chunk| chunk.text())
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
pub(crate) mod tests {
    use super::*;

    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = id.to_vec();
        out.extend((body.len() as u32).to_be_bytes());
        out.extend(body);
        if body.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    fn list(kind: &[u8; 4], children: &[Vec<u8>]) -> Vec<u8> {
        let mut body = kind.to_vec();
        body.extend(children.concat());
        chunk(b"LIST", &body)
    }

    fn name(text: &str) -> Vec<u8> {
        let mut body = text.as_bytes().to_vec();
        body.resize(40, 0);
        chunk(b"tdmn", &body)
    }

    fn path(steps: &[&str]) -> Vec<u8> {
        let steps: Vec<Vec<u8>> = steps
            .iter()
            .map(|step| list(b"tdsi", &[chunk(b"tdix", &[0; 4]), name(step)]))
            .collect();
        list(b"tdsp", &steps)
    }

    /// One slider, as an effect stores it: a definition in `parT` saying it is
    /// a slider and what it is called, and a value in the group.
    pub(crate) struct Slider<'a> {
        pub label: &'a str,
        pub value: f64,
        pub expression: Option<&'a str>,
    }

    /// One effect's `LIST sspc`, holding sliders numbered from one.
    fn effect(match_name: &str, display: &str, sliders: &[Slider<'_>]) -> Vec<u8> {
        let mut definitions = vec![chunk(b"parn", &(sliders.len() as u32 + 1).to_be_bytes())];
        let mut group = vec![chunk(b"tdsb", &[0, 0, 0, 1])];
        for (i, slider) in sliders.iter().enumerate() {
            let id = format!("{match_name}-{:04}", i + 1);
            let mut pard = vec![0u8; 148];
            pard[15] = 10; // a float slider
            pard[16..16 + slider.label.len()].copy_from_slice(slider.label.as_bytes());
            definitions.push(name(&id));
            definitions.push(chunk(b"pard", &pard));

            let mut tdb4 = vec![0u8; 124];
            tdb4[..2].copy_from_slice(&[0xdb, 0x99]);
            tdb4[3] = 1; // one dimension
            let mut leaf = vec![
                chunk(b"tdsb", &[0, 0, 0, 1]),
                chunk(b"tdb4", &tdb4),
                chunk(b"cdat", &slider.value.to_be_bytes()),
            ];
            if let Some(expression) = slider.expression {
                leaf.push(chunk(b"Utf8", expression.as_bytes()));
            }
            group.push(name(&id));
            group.push(list(b"tdbs", &leaf));
        }
        list(
            b"sspc",
            &[
                chunk(b"fnam", &chunk(b"Utf8", display.as_bytes())),
                list(b"parT", &definitions),
                list(b"tdgp", &group),
            ],
        )
    }

    /// A whole preset file holding these effects, saved from a 1280 × 720
    /// layer.
    pub(crate) fn preset_bytes(effects: &[(&str, &str, &[Slider<'_>])]) -> Vec<u8> {
        let mut beso = vec![0u8; 56];
        beso[12..16].copy_from_slice(&24576u32.to_be_bytes());
        for (at, side) in [(24, 1280u16), (26, 720), (28, 1280), (30, 720)] {
            beso[at..at + 2].copy_from_slice(&side.to_be_bytes());
        }
        let mut inside = vec![chunk(b"beso", &beso)];
        for (match_name, display, _) in effects {
            inside.push(path(&[EFFECT_PARADE, match_name]));
            inside.push(chunk(b"tdsn", display.as_bytes()));
        }
        inside.push(path(&[SENTINEL]));
        for (match_name, display, sliders) in effects {
            inside.push(effect(match_name, display, sliders));
        }
        let mut body = b"FaFX".to_vec();
        body.extend(chunk(b"head", &[0; 16]));
        body.extend(list(b"besc", &inside));
        let mut file = b"RIFX".to_vec();
        file.extend((body.len() as u32).to_be_bytes());
        file.extend(body);
        file
    }

    /// A preset's effects come out in order, each with its own name, its rows'
    /// names and their expressions, and a file that is not a preset is refused.
    #[test]
    fn a_presets_effects_are_read_in_order() {
        let bytes = preset_bytes(&[
            (
                "Pseudo/1",
                "Shake",
                &[Slider {
                    label: "Amount",
                    value: 12.5,
                    expression: None,
                }],
            ),
            (
                "ADBE Slider Control",
                "Driven",
                &[Slider {
                    label: "Slider",
                    value: 3.0,
                    expression: Some("effect(\"Shake\")(\"Amount\") * 2"),
                }],
            ),
        ]);
        assert!(is_preset(&bytes));
        let preset = parse_preset(&bytes).unwrap();
        assert_eq!(preset.size, (1280.0, 720.0));
        assert!(preset.skipped.is_empty(), "{:?}", preset.skipped);

        let names: Vec<_> = preset
            .effects
            .iter()
            .map(|e| (e.match_name.as_deref().unwrap(), e.name.as_deref().unwrap()))
            .collect();
        assert_eq!(
            names,
            [("Pseudo/1", "Shake"), ("ADBE Slider Control", "Driven")]
        );
        let amount = &preset.effects[0].children()[0];
        assert_eq!(amount.name.as_deref(), Some("Amount"));
        assert_eq!(amount.value, Some(serde_json::json!(12.5)));
        assert_eq!(amount.control.as_deref(), Some("slider"));
        let driven = &preset.effects[1].children()[0];
        assert_eq!(
            driven.expression.as_deref(),
            Some("effect(\"Shake\")(\"Amount\") * 2")
        );
        assert_eq!(driven.expression_enabled, Some(true));

        assert!(!is_preset(b"RIFX\0\0\0\x04Egg!"));
        assert!(parse_preset(b"RIFX\0\0\0\x04Egg!").is_err());
        assert!(parse_preset(&bytes[..40]).is_err());
    }
}
