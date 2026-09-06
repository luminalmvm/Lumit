//! Every shipped WGSL kernel parses and passes validation.
//!
//! **In plain terms.** The shaders are the little programs that run on the
//! graphics card. Until now the only thing that ever checked them was the card
//! itself, at the moment the pipeline was built — so a typo in a shader
//! compiled fine, shipped fine, and turned up as a black Viewer on a machine
//! with a graphics card, and never at all on a machine or a CI runner without
//! one. This test runs the same front end wgpu runs (`naga`, the shader
//! compiler wgpu is built on) over every `.wgsl` file in the crate and fails on
//! anything it would reject. No graphics card involved, so it runs everywhere
//! and it runs fast.
//!
//! It does NOT check that a shader does the right thing — that is what the
//! CPU-oracle tests in `fx/tests.rs` are for, and those need a card.

use std::path::Path;

/// The shared noise core, exactly as `FxEngine::new` prepends it.
const NOISE_CORE: &str = include_str!("../src/fx_noise_core.wgsl");
/// The shared OCIO sampler, exactly as both its consumers prepend it.
const OCIO_SAMPLE: &str = include_str!("../src/ocio_sample.wgsl");

#[test]
fn every_wgsl_kernel_parses_and_validates() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut checked = 0usize;
    let mut failures = Vec::new();
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "wgsl"))
        .collect();
    // Sorted so a failure list reads the same on every machine.
    paths.sort();
    for path in paths {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        let Ok(source) = std::fs::read_to_string(&path) else {
            failures.push(format!("{name}: unreadable"));
            continue;
        };
        // `fx_noise_core.wgsl` is not a kernel: it is prepended to the kernels
        // that read the noise field, which is WGSL's only way of having a shared
        // module (docs/08 §3.37, §3.38). So it is validated exactly as the engine
        // compiles it — as part of each consumer — rather than on its own, where
        // it has no entry point and its consumers have no `nc_fractal`.
        if name == "fx_noise_core.wgsl" || name == "ocio_sample.wgsl" {
            continue;
        }
        // Any `nc_` name means this kernel is one of the core's consumers —
        // Card wipe (docs/08 §3.72) takes only `nc_hash01` and no fractal at
        // all, so the older "does it call nc_fractal" test would have compiled
        // it alone and failed on an unknown identifier.
        let source = if source.contains("nc_") {
            format!("{NOISE_CORE}{source}")
        } else {
            source
        };
        // The OCIO sampler is the second shared module, prepended to the
        // colour pipeline and to the OCIO effects' kernel the same way.
        let source = if source.contains("ocio_apply(") {
            format!("{OCIO_SAMPLE}{source}")
        } else {
            source
        };
        let module = match naga::front::wgsl::parse_str(&source) {
            Ok(module) => module,
            Err(e) => {
                failures.push(format!("{name}: parse: {}", e.emit_to_string(&source)));
                continue;
            }
        };
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            // The capabilities wgpu's default device gives a kernel; a shader
            // that needs more than this would fail at pipeline creation on a
            // stock adapter, which is exactly what this catches.
            naga::valid::Capabilities::empty(),
        );
        if let Err(e) = validator.validate(&module) {
            failures.push(format!("{name}: validate: {}", e.emit_to_string(&source)));
            continue;
        }
        checked += 1;
    }
    assert!(
        failures.is_empty(),
        "invalid WGSL:\n{}",
        failures.join("\n")
    );
    // A guard against the test quietly checking nothing (a moved directory, a
    // filter that stops matching): the crate has dozens of kernels.
    assert!(
        checked >= 20,
        "expected the crate's WGSL kernels to be found, validated {checked}"
    );
}
