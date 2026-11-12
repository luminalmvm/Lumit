//! Tests for refusing a composition size no picture could be.

use super::*;

/// A composition's size decides every raster allocation made for it, and it
/// arrives as two plain integers out of the file's JSON — which a text
/// editor can set to anything.
///
/// Refused rather than clamped: a project opened quietly at a size other
/// than the one it states would *export* at that size too, and a wrong
/// picture delivered quietly is the failure this codebase refuses to ship.
#[test]
fn a_composition_no_picture_could_be_is_refused_rather_than_opened() {
    fn comp_sized(width: u32, height: u32) -> lumit_core::model::Composition {
        lumit_core::model::Composition {
            graph: None,
            master_volume_db: 0.0,
            lighting: Default::default(),
            sound_mix: false,
            groups: Vec::new(),
            beat_grid: None,
            id: Uuid::now_v7(),
            name: "Comp 1".into(),
            width,
            height,
            frame_rate: lumit_core::time::FrameRate::new(25, 1).unwrap(),
            duration: lumit_core::time::Duration(lumit_core::time::Rational::new(10, 1).unwrap()),
            background: lumit_core::model::LinearColour::BLACK,
            work_area: None,
            layers: Vec::new(),
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let saved = |w: u32, h: u32, name: &str| {
        let mut doc = Document::default();
        doc.items
            .push(lumit_core::model::ProjectItem::Composition(comp_sized(
                w, h,
            )));
        let at = dir.path().join(name);
        save(&doc, &at).unwrap();
        at
    };

    assert!(
        open(&saved(1920, 1080, "honest.lum")).is_ok(),
        "an ordinary project opens"
    );

    for (w, h) in [
        (u32::MAX, 1080),
        (1920, u32::MAX),
        // Each side inside the ceiling, the product far past it.
        (65_536, 65_536),
        (0, 1080),
        (1920, 0),
    ] {
        let at = saved(w, h, &format!("{w}x{h}.lum"));
        let out = open(&at);
        assert!(
            matches!(out, Err(ProjectError::CompTooLarge { .. })),
            "{w} by {h} must be refused, got {out:?}"
        );
    }

    // And the predicate agrees with the sizes people actually work at.
    for (w, h) in [(1920, 1080), (3840, 2160), (15_360, 8640), (1, 1)] {
        assert!(comp_size_is_sane(w, h), "{w} by {h} is an ordinary comp");
    }
}
