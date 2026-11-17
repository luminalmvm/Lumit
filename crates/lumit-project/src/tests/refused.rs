//! Files refused rather than opened.

use super::*;

/// A `.lum` is a ZIP, and a ZIP entry's compressed size says nothing about
/// its decompressed one: a few hundred kilobytes of deflated zeros expand to
/// gigabytes. `read_to_string` believes whatever comes out of the
/// decompressor — so a project file somebody sent, which is what project
/// files are for, could take the machine's memory before a single field had
/// been parsed.
#[test]
fn a_project_entry_that_decompresses_without_end_is_refused() {
    /// A reader that claims one size and produces another — which is the
    /// whole shape of the attack, and not something a real archive library
    /// will do on request.
    struct Liar {
        claims: u64,
        produced: usize,
    }
    impl std::io::Read for Liar {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            // Endless, like a decompressor fed a bomb.
            buf.fill(b'x');
            self.produced = self.produced.saturating_add(buf.len());
            Ok(buf.len())
        }
    }
    impl ZipEntry for Liar {
        fn size(&self) -> u64 {
            self.claims
        }
    }

    // Honest and over the ceiling: refused from the header, having read
    // nothing.
    let out = entry_text(
        Liar {
            claims: 1 << 40,
            produced: 0,
        },
        1024,
    );
    assert!(
        matches!(out, Err(ProjectError::EntryTooLarge { limit: 1024, .. })),
        "{out:?}"
    );

    // Understating its size and then never stopping: refused on the byte
    // past the ceiling, rather than read until the memory ran out.
    let out = entry_text(
        Liar {
            claims: 8,
            produced: 0,
        },
        1024,
    );
    assert!(
        matches!(out, Err(ProjectError::EntryTooLarge { limit: 1024, .. })),
        "a stream that lied about its size must be caught by what it produced: {out:?}"
    );

    // And an ordinary entry is unaffected.
    let honest = entry_text(
        {
            struct Short(&'static [u8]);
            impl std::io::Read for Short {
                fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                    std::io::Read::read(&mut self.0, buf)
                }
            }
            impl ZipEntry for Short {
                fn size(&self) -> u64 {
                    3
                }
            }
            Short(b"{}\n")
        },
        1024,
    )
    .expect("an ordinary entry is read");
    assert_eq!(honest.trim(), "{}");
}

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
            master_keys: None,
            master_automation: Default::default(),
            loudness_target: None,
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
