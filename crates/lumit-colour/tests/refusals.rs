//! The refusal taxonomy (docs/impl/ocio.md §7.6).
//!
//! In plain terms: `refusals/` is a folder of configs that each ask for exactly
//! one thing Lumit does not do. This test walks them and insists that every one
//! **refuses, and refuses by the right name** — the transform, the style, the
//! space, the file. Silence is the failure mode this whole design exists to
//! avoid: a config that half-works produces a picture that looks plausible and
//! is wrong, which nobody catches until a delivery is rejected.
//!
//! Adding a case is adding a file. The first line says what the refusal must
//! say: `# refuses: <text the message must contain>`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use lumit_colour::resolve::{all_views, unresolvable};
use lumit_colour::{Config, LoadedConfig};

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/refusals")
}

/// Every refusal a config produces, from parsing it right through to walking
/// every space and every view. One of them must name the expected thing.
fn refusals(dir: &Path, text: &str) -> Vec<String> {
    let config = match Config::parse(dir, text) {
        Ok(config) => config,
        Err(e) => return vec![e.to_string()],
    };
    let loaded = LoadedConfig::new(config);
    let mut out: Vec<String> = unresolvable(&loaded)
        .into_iter()
        .map(|(name, e)| format!("{name}: {e}"))
        .collect();
    // A space may resolve one way and refuse the other, and a view is its own
    // walk, so both directions and every view are asked.
    for name in loaded.config.space_order.clone() {
        if let Err(e) = loaded.from_reference(&name) {
            out.push(format!("{name}: {e}"));
        }
    }
    for (display, view) in all_views(&loaded.config) {
        if let Err(e) = loaded.display_view(display, &view.name) {
            out.push(format!("{display}/{}: {e}", view.name));
        }
    }
    out
}

#[test]
fn every_unsupported_config_refuses_by_name() {
    let dir = corpus();
    let entries = std::fs::read_dir(&dir).expect("the refusal corpus is readable");
    let mut checked = 0;
    for entry in entries {
        let path = entry.expect("a readable entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("ocio") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("a readable config");
        let expected = text
            .lines()
            .next()
            .and_then(|l| l.trim().strip_prefix("# refuses:"))
            .map(str::trim)
            .unwrap_or_else(|| panic!("{path:?} has no '# refuses:' first line"))
            .to_string();

        let found = refusals(&dir, &text);
        assert!(
            !found.is_empty(),
            "{path:?} was accepted; it must refuse with {expected:?}"
        );
        assert!(
            found.iter().any(|m| m.contains(&expected)),
            "{path:?} refused, but not by name.\n  expected to contain: {expected}\n  got: {found:#?}"
        );
        checked += 1;
    }
    // A corpus that quietly emptied itself would pass every assertion above.
    assert!(checked >= 14, "only {checked} refusal cases were walked");
}

/// The other half of the promise: a config using only what Lumit implements
/// must resolve cleanly. Without this, "refuse everything" would pass the test
/// above.
#[test]
fn a_config_inside_the_implemented_set_does_not_refuse() {
    let text = r#"
ocio_profile_version: 2
search_path: luts
roles:
  scene_linear: lin
  aces_interchange: ACES2065-1
displays:
  sRGB:
    - !<View> {name: Standard, colorspace: out_srgb}
colorspaces:
  - !<ColorSpace>
    name: ACES2065-1
  - !<ColorSpace>
    name: lin
    to_scene_reference: !<BuiltinTransform> {style: ACEScg_to_ACES2065-1}
  - !<ColorSpace>
    name: cct
    to_scene_reference: !<BuiltinTransform> {style: ACEScct_to_ACES2065-1}
  - !<ColorSpace>
    name: out_srgb
    from_scene_reference: !<ExponentWithLinearTransform> {gamma: [2.4, 2.4, 2.4, 1], offset: [0.055, 0.055, 0.055, 0], direction: inverse}
  # Declared in both directions, because a 3D table cannot be inverted and a
  # space that states only one side would rightly refuse the other (§4.3).
  - !<ColorSpace>
    name: cubed
    to_scene_reference: !<FileTransform> {src: tiny.spi3d}
    from_scene_reference: !<FileTransform> {src: tiny.spi3d}
"#;
    let found = refusals(&corpus(), text);
    assert!(found.is_empty(), "a supported config refused: {found:#?}");
}
