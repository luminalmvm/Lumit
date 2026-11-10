use super::*;

/// The colour-blind form of [`super::spec_check`], for the rows that have
/// nothing to do with colour management: a project naming no config, which
/// is what every test below is. Every row that asks it is about what a
/// container can carry, so they are all `media` rows and so is this.
#[cfg(feature = "media")]
fn spec_check(spec: &BridgeExportSpec) -> String {
    super::spec_check(spec, &ColourState::default())
}

/// A zero size, a zero rate and a negative range all mean "the
/// composition's own", which is what the dialogue's untouched fields say —
/// never a frame of 0×0 or a range of nothing.
#[test]
#[cfg(feature = "media")]
fn the_untouched_fields_mean_the_composition_s_own() {
    let resolved = to_export_spec(&BridgeExportSpec::default(), 640, 360).unwrap();
    assert_eq!(resolved.target, None, "zero size is the comp's own frame");
    assert_eq!(resolved.fps, None, "zero rate is the comp's own");
    assert_eq!(resolved.range, None, "a negative range is the work area");

    let spec = BridgeExportSpec {
        width: 1920,
        height: 1080,
        fps: 29.97,
        range_start_frame: 12,
        range_end_frame: 48,
        ..BridgeExportSpec::default()
    };
    let resolved = to_export_spec(&spec, 640, 360).unwrap();
    assert_eq!(resolved.target, Some((1920, 1080)));
    assert_eq!(resolved.fps, Some(29.97));
    assert_eq!(resolved.range, Some((12, 48)));

    // A backwards range is no range rather than a clamped one: it is a slip
    // of the fingers, not an instruction.
    let backwards = BridgeExportSpec {
        range_start_frame: 48,
        range_end_frame: 12,
        ..BridgeExportSpec::default()
    };
    assert_eq!(to_export_spec(&backwards, 640, 360).unwrap().range, None);
}

/// A spec the format cannot honour is refused in the engine's own words,
/// before anything is queued — and an exportable one says nothing.
#[test]
#[cfg(feature = "media")]
fn a_format_refuses_what_it_cannot_carry() {
    assert_eq!(spec_check(&BridgeExportSpec::default()), "");

    let deep_mp4 = BridgeExportSpec {
        depth: 16,
        ..BridgeExportSpec::default()
    };
    assert!(
        spec_check(&deep_mp4).contains("16-bit"),
        "an mp4 says it cannot carry 16-bit colour rather than writing 8"
    );

    let alpha_mp4 = BridgeExportSpec {
        alpha_channel: true,
        ..BridgeExportSpec::default()
    };
    assert!(spec_check(&alpha_mp4).contains("alpha"));

    // The same two settings in a PNG sequence are perfectly ordinary.
    let stills = BridgeExportSpec {
        codec: "png".to_owned(),
        depth: 16,
        alpha_channel: true,
        ..BridgeExportSpec::default()
    };
    assert_eq!(spec_check(&stills), "");
}

/// The crop is the typed insets, or the Viewer's region when that is asked
/// for — and the reading is the frame that survives it.
#[test]
fn the_crop_answers_the_typed_insets_or_the_region() {
    let typed = BridgeExportSpec {
        crop_top: 10,
        crop_left: 20,
        crop_bottom: 30,
        crop_right: 40,
        ..BridgeExportSpec::default()
    };
    let crop = crop_for(&typed, 1000, 500);
    assert_eq!(
        (crop.top, crop.left, crop.bottom, crop.right),
        (10, 20, 30, 40)
    );
    assert_eq!((crop.width, crop.height), (940, 460));

    // The region wins when it is ticked and set.
    let region = BridgeExportSpec {
        use_region_of_interest: true,
        region: vec![0.25, 0.0, 0.75, 1.0],
        ..typed.clone()
    };
    let crop = crop_for(&region, 1000, 500);
    assert_eq!((crop.left, crop.right), (250, 250));
    assert_eq!((crop.width, crop.height), (500, 500));

    // A degenerate region is no region, and the typed crop stands.
    let degenerate = BridgeExportSpec {
        region: vec![0.5, 0.5, 0.5, 0.5],
        ..region
    };
    assert_eq!(crop_for(&degenerate, 1000, 500).left, 20);
}

/// A preset saved from the dialogue comes back as the settings that saved
/// it — every field, not the eight the seam used to carry. The engine's own
/// store is exercised by `export_presets`; this is the conversion round
/// trip either side of it.
#[test]
#[cfg(feature = "media")]
fn a_spec_survives_the_round_trip_through_the_engine_s_own() {
    let spec = BridgeExportSpec {
        preset: String::new(),
        codec: "tiff".to_owned(),
        width: 1280,
        height: 720,
        bitrate_mbps: 0,
        peak_mbps: 0,
        bitrate_auto: false,
        fps: 24.0,
        range_start_frame: 5,
        range_end_frame: 25,
        include_audio: false,
        audio_bit_rate: 192_000,
        depth: 16,
        alpha_channel: true,
        straight_alpha: true,
        colour_space: String::new(),
        crop_top: 1,
        crop_left: 2,
        crop_bottom: 3,
        crop_right: 4,
        use_region_of_interest: false,
        region: Vec::new(),
        metadata: vec![
            BridgeMetadataField {
                key: "title".to_owned(),
                value: "Scene 1".to_owned(),
            },
            BridgeMetadataField {
                key: "artist".to_owned(),
                value: "Nobody".to_owned(),
            },
        ],
        quality_divisor: 2,
        disk_cache_read_only: true,
        effects: false,
        honour_solo: false,
        // The sound settings come back *resolved*, never as the zero that
        // asked for the default: a preset the dialogue loads has to fill a
        // dropdown, and "whatever the default is" selects nothing. So the
        // round trip is exact only from a spec that states them, which is
        // exactly what a saved preset is.
        audio_rate: 48_000,
        audio_depth: 16,
        audio_channels: 2,
        ..BridgeExportSpec::default()
    };
    let resolved = to_export_spec(&spec, 1920, 1080).unwrap();
    let back = from_export_spec(&resolved).unwrap();
    assert_eq!(back, spec);
    assert_eq!(
        resolved.metadata.iter().map(|(k, _)| k).collect::<Vec<_>>(),
        ["title", "artist"],
        "metadata keeps the order it was given — the order lands in the file"
    );
}

/// The capability rows the dialogue reads are the engine's own answers, and
/// the two the seam *flattens* cannot drift from the lists they flatten.
///
/// `audio_24_bit` is a flag standing in for a two-member list and
/// `export_audio_rates` is one list standing in for every format's, so both
/// are asserted against `FormatCaps` itself for every format there is —
/// which is what makes the flattening safe rather than merely tidy.
#[test]
#[cfg(feature = "media")]
fn the_caps_row_says_what_the_engine_says() {
    use lumit_render::export::{AudioDepth, EXPORT_AUDIO_RATES};

    assert_eq!(audio_rates(), EXPORT_AUDIO_RATES.to_vec());

    for codec in [
        "h264",
        "hevc",
        "prores_proxy",
        "prores_lt",
        "prores_422",
        "prores_hq",
        "prores_4444",
        "prores_4444xq",
        "png",
        "tiff",
        "m4a",
        "wav",
    ] {
        let caps = format_caps(codec);
        let engine = export_format(codec).unwrap().caps();

        assert_eq!(
            caps.audio_24_bit,
            engine.audio_depths.contains(&AudioDepth::TwentyFour),
            "{codec}: the 24-bit flag is the engine's own depth list"
        );
        // The flag only stands in for the list while the list has two
        // members; the moment a third width exists this row must become a
        // `Vec<u32>` like `depths` (the ponytail note on the field).
        assert!(
            engine.audio_depths.len() <= AudioDepth::ALL.len() && AudioDepth::ALL.len() == 2,
            "{codec}: a third sample width means the flag is no longer enough"
        );

        // Rates do not vary by format: a format either carries sound and
        // carries all of them, or carries none — which is what lets
        // `export_audio_rates` be one call rather than a caps row.
        if engine.audio {
            assert_eq!(engine.audio_rates, EXPORT_AUDIO_RATES, "{codec}");
        } else {
            assert!(engine.audio_rates.is_empty(), "{codec}");
        }

        assert_eq!(
            caps.colour_spaces,
            engine
                .colour_spaces
                .iter()
                .map(lumit_render::export::ColourSpace::stored_name)
                .collect::<Vec<_>>(),
            "{codec}: the colour list is the engine's, by stored name"
        );
    }

    // The names are the ones `colour_space` is set with, and the default
    // space is the empty string so a spec that never sets one is the space
    // every export has always written in.
    assert!(format_caps("h264").colour_spaces.contains(&String::new()));
    assert!(format_caps("h264")
        .colour_spaces
        .contains(&"rec2020".to_owned()));
    assert!(format_caps("png").colour_spaces == vec![String::new()]);
    assert!(format_caps("wav").colour_spaces.is_empty());
}

/// Keep both never writes over a file: not the export's own name, and not
/// frame one of a sequence under a numbered name either.
#[test]
#[cfg(feature = "media")]
fn keep_both_finds_a_name_that_writes_over_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let at = |name: &str| dir.path().join(name).to_string_lossy().into_owned();
    std::fs::write(at("shot.mp4"), b"").unwrap();
    std::fs::write(at("shot_2.mp4"), b"").unwrap();
    let burn = BridgeCaptionsOut::Burn;
    assert!(path_taken(&at("shot.mp4"), "h264", burn));
    assert_eq!(free_path(&at("shot.mp4"), "h264", burn), at("shot_3.mp4"));

    std::fs::write(at("frames.00001.png"), b"").unwrap();
    assert!(path_taken(&at("frames.png"), "png", burn));
    assert_eq!(
        free_path(&at("frames.png"), "png", burn),
        at("frames_2.png")
    );
    assert!(!path_taken(&at("other.png"), "png", burn));

    // A caption file beside it counts when the export writes one.
    std::fs::write(at("talk.en.srt"), b"").unwrap();
    assert!(!path_taken(&at("talk.mp4"), "h264", burn));
    assert!(path_taken(&at("talk.mp4"), "h264", BridgeCaptionsOut::Srt));
    assert!(!path_taken(&at("talk.mp4"), "h264", BridgeCaptionsOut::Vtt));
}
