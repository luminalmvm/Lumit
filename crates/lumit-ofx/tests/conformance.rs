//! The conformance pass: every plugin on this machine, every context this host
//! drives, describe to rendered frame (docs/impl/ofx-host.md §5 item 1).
//!
//! # In plain terms
//!
//! The unit tests prove the host against a plugin Lumit wrote, which is a
//! plugin that agrees with us about everything. This one proves it against
//! plugins that do not: **openfx-misc** (Natron's set, some eighty of them) and
//! **ntsc-rs**, both free, both named by docs/12 §2.5. Each plugin is asked to
//! describe itself, asked again once per context it declares, made into an
//! instance, and handed a frame; what comes back is checked, and the whole run
//! becomes a table of passes, rejections and failures.
//!
//! Three things it asserts that no other test can:
//!
//! * **No suite call was refused.** The host tallies every status it answers
//!   ([`lumit_ofx::status::answered`]), so `kOfxStatErrBadHandle` or
//!   `kOfxStatErrValue` coming back from any call during the pass is a failure
//!   even when the picture looked right. A plugin swallows those codes and
//!   carries on; that is exactly how a host bug hides.
//! * **The output is a picture**: the right number of pixels, every one of them
//!   finite. A NaN in a plugin's output is the host's problem the moment it
//!   reaches a composite.
//! * **A rejection is not a failure.** This host is fp32 RGBA, full-frame, no
//!   tiles, two contexts. A plugin that legitimately cannot work in it is
//!   *rejected at describe* with a reason, and the reason is a row in the table
//!   (docs/12 §2.1).
//!
//! # Running it
//!
//! With no bench on the machine it runs against Lumit's own test plugin and
//! says so — a developer who has not built eighty plugins still gets a green
//! suite. To run the real thing, point `LUMIT_OFX_BENCH` at a folder of
//! bundles, or let `cargo run -p lumit-bench --bin ofx-bench` fetch and build
//! them into the default folder first.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use lumit_ofx::bundle::{Bundle, BUNDLE_ARCH_DIR};
use lumit_ofx::describe::{describe, describe_in, Context};
use lumit_ofx::image::{Frame16, RectI, RowOrder};
use lumit_ofx::instance::{Instance, ParamSnapshot};
use lumit_ofx::render::{RenderRequest, OUTPUT_CLIP};
use lumit_ofx::schema::schema_of;
use lumit_ofx::status::{answered, forget, Status};

/// The fixture frame's size. Big enough that a plugin's blur has somewhere to
/// go, small enough that eighty plugins are seconds rather than minutes.
const FIXTURE: (usize, usize) = (64, 48);

/// Where the bench's bundles are looked for when `LUMIT_OFX_BENCH` says
/// nothing. The same default `lumit-bench`'s runner builds into, so the two
/// need no arrangement beyond this constant.
fn default_bench_dir() -> PathBuf {
    std::env::temp_dir().join("lumit-ofx-bench")
}

/// What happened to one plugin in one context.
enum Outcome {
    /// It described, instanced and rendered.
    Passed {
        /// Whether the frame that came back differs from the one that went in.
        /// Not asserted — a plugin at its defaults is often an identity — but
        /// reported, because a whole bench that changed nothing means the
        /// pictures never reached anybody.
        changed: bool,
        /// Whether the plugin short-circuited the render by answering
        /// `isIdentity`.
        identity: bool,
    },
    /// It cannot be an effect in this host, and said why. Not a failure.
    Rejected(String),
    /// It should have worked and did not.
    Failed(String),
}

/// One row of the table.
struct Row {
    bundle: String,
    identifier: String,
    version: (u32, u32),
    context: &'static str,
    outcome: Outcome,
}

/// The test plugin's file name on this platform.
fn test_plugin_file_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "lumit_ofx_testplug.dll"
    } else if cfg!(target_os = "macos") {
        "liblumit_ofx_testplug.dylib"
    } else {
        "liblumit_ofx_testplug.so"
    }
}

/// Where Cargo put the test plugin, if it built it.
fn test_plugin() -> Option<PathBuf> {
    let name = test_plugin_file_name();
    let exe = std::env::current_exe().ok()?;
    let mut dir = exe.parent()?;
    for _ in 0..3 {
        for candidate in [dir.join(name), dir.join("deps").join(name)] {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        dir = dir.parent()?;
    }
    None
}

/// Lay the test plugin out as a real bundle inside `root`.
fn a_fixture_bundle(root: &Path) -> Option<PathBuf> {
    let source = test_plugin()?;
    let dir = root
        .join("Lumit.ofx.bundle")
        .join("Contents")
        .join(BUNDLE_ARCH_DIR);
    std::fs::create_dir_all(&dir).ok()?;
    let binary = dir.join("Lumit.ofx");
    std::fs::copy(&source, &binary).ok()?;
    Some(binary)
}

/// Every bundle binary under the bench folders, and the folders themselves, so
/// a message can name where nothing was found.
fn bench_bundles() -> (Vec<PathBuf>, Vec<PathBuf>) {
    let dirs: Vec<PathBuf> = match std::env::var_os("LUMIT_OFX_BENCH") {
        Some(value) => std::env::split_paths(&value).collect(),
        None => vec![default_bench_dir()],
    };
    let found = dirs
        .iter()
        .filter(|dir| dir.is_dir())
        .flat_map(|dir| lumit_ofx::bundle::scan_dir(dir))
        .collect();
    (found, dirs)
}

/// A picture that says where every pixel is, so a flipped or mirrored frame is
/// obvious rather than plausible, with a highlight above one because the
/// working space is scene-linear (docs/08 §2.1).
fn fixture_frame() -> Frame16 {
    let (width, height) = FIXTURE;
    let mut pixels = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            pixels.push(half::f16::from_f32(x as f32 / width as f32));
            pixels.push(half::f16::from_f32(y as f32 / height as f32));
            pixels.push(half::f16::from_f32(1.5));
            pixels.push(half::f16::ONE);
        }
    }
    Frame16::from_pixels(width, height, pixels).expect("the count matches the size")
}

/// One frame's worth of question for a plugin with these clips: a picture for
/// every input it defined, at the fixture size.
fn request_for(clips: &[String], source: &Frame16) -> RenderRequest {
    let (width, height) = FIXTURE;
    let mut inputs = BTreeMap::new();
    for name in clips.iter().filter(|name| name.as_str() != OUTPUT_CLIP) {
        inputs.insert(name.clone(), source.clone());
    }
    // A plugin that defined no input at all (a generator-shaped general effect)
    // still gets the standard name, so the render has something to be identity
    // of if it says it is.
    if inputs.is_empty() {
        inputs.insert("Source".to_owned(), source.clone());
    }
    RenderRequest {
        time: 0.0,
        bounds: RectI::sized(width as i32, height as i32),
        order: RowOrder::TopDown,
        inputs,
    }
}

/// Drive one plugin in one context, all the way to a frame.
fn drive(
    plugin: &lumit_ofx::bundle::PluginRef,
    context: Context,
    source: &Frame16,
) -> Result<Outcome, Outcome> {
    let descriptor = match describe_in(plugin, Some(context)) {
        Ok(descriptor) => descriptor,
        Err(reason) => return Ok(Outcome::Rejected(reason.to_string())),
    };
    // Becoming an effect is part of the pass: a plugin the host can drive but
    // Lumit cannot declare is rejected here rather than at the first click
    // (docs/impl/ofx-host.md §4a).
    if let Err(reason) = schema_of(&descriptor) {
        return Ok(Outcome::Rejected(reason.to_string()));
    }

    let instance = match Instance::create(plugin, &descriptor, context, &ParamSnapshot::new()) {
        Ok(instance) => instance,
        Err(status) => {
            return Err(Outcome::Failed(format!(
                "createInstance answered {status:?}"
            )))
        }
    };
    let clips: Vec<String> = descriptor.clips.iter().map(|c| c.name.clone()).collect();
    let request = request_for(&clips, source);
    let token = lumit_eval::epoch::Epoch::new().token();
    let rendered = lumit_ofx::render::render(plugin, &instance, &request, &token);
    let _ = instance.destroy(plugin);

    let rendered = match rendered {
        Ok(rendered) => rendered,
        Err(error) => return Err(Outcome::Failed(error.to_string())),
    };

    let (width, height) = FIXTURE;
    let frame = &rendered.frame;
    if frame.width() != width || frame.height() != height {
        return Err(Outcome::Failed(format!(
            "the frame came back {}x{} instead of {width}x{height}",
            frame.width(),
            frame.height()
        )));
    }
    if let Some(at) = frame
        .pixels()
        .iter()
        .position(|value| !f32::from(*value).is_finite())
    {
        return Err(Outcome::Failed(format!(
            "the frame carries a value that is not a number, at element {at}"
        )));
    }
    let changed = frame.pixels() != source.pixels();
    Ok(Outcome::Passed {
        changed,
        identity: rendered.identity_of.is_some(),
    })
}

/// Every plugin in one bundle, in every context it declares.
fn drive_bundle(path: &Path, source: &Frame16, rows: &mut Vec<Row>) {
    let label = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into(),
    );
    let Ok(mut bundle) = Bundle::open(path) else {
        rows.push(Row {
            bundle: label,
            identifier: "-".to_owned(),
            version: (0, 0),
            context: "-",
            outcome: Outcome::Rejected("the bundle would not open".to_owned()),
        });
        return;
    };
    bundle.load();

    for plugin in bundle.plugins() {
        if !plugin.is_supported_image_effect() {
            continue;
        }
        // The first describe is what says which contexts there are to drive;
        // a plugin that cannot even do that is one rejected row, not one per
        // context it never named.
        let contexts = match describe(plugin) {
            Ok(descriptor) => descriptor.contexts,
            Err(reason) => {
                rows.push(Row {
                    bundle: label.clone(),
                    identifier: plugin.identifier.clone(),
                    version: plugin.version,
                    context: "-",
                    outcome: Outcome::Rejected(reason.to_string()),
                });
                continue;
            }
        };
        for context in contexts {
            let outcome = match drive(plugin, context, source) {
                Ok(outcome) | Err(outcome) => outcome,
            };
            rows.push(Row {
                bundle: label.clone(),
                identifier: plugin.identifier.clone(),
                version: plugin.version,
                context: context.ofx_name(),
                outcome,
            });
        }
    }
    bundle.unload();
}

/// The table, as the markdown that goes in the pull request.
fn table(rows: &[Row], bad_handle: u64, bad_value: u64) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "| bundle | plugin | version | context | result |");
    let _ = writeln!(out, "|---|---|---|---|---|");
    for row in rows {
        let result = match &row.outcome {
            Outcome::Passed { changed, identity } => {
                let mut what = "passed".to_owned();
                if *identity {
                    what.push_str(" (isIdentity)");
                } else if !*changed {
                    what.push_str(" (unchanged)");
                }
                what
            }
            Outcome::Rejected(why) => format!("rejected at describe — {why}"),
            Outcome::Failed(why) => format!("**FAILED** — {why}"),
        };
        let _ = writeln!(
            out,
            "| {} | {} | {}.{} | {} | {} |",
            row.bundle, row.identifier, row.version.0, row.version.1, row.context, result
        );
    }
    let passed = rows
        .iter()
        .filter(|r| matches!(r.outcome, Outcome::Passed { .. }))
        .count();
    let rejected = rows
        .iter()
        .filter(|r| matches!(r.outcome, Outcome::Rejected(_)))
        .count();
    let failed = rows.len() - passed - rejected;
    let _ = writeln!(
        out,
        "\n{passed} passed, {rejected} rejected at describe, {failed} failed; \
         the host answered kOfxStatErrBadHandle {bad_handle} times and \
         kOfxStatErrValue {bad_value} times during the pass."
    );
    out
}

/// Where the table is written, so CI can put it in the pull request body.
fn out_path() -> PathBuf {
    if let Some(named) = std::env::var_os("OFX_CONFORMANCE_OUT") {
        return PathBuf::from(named);
    }
    // target/debug/deps/conformance-<hash> → target/
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.ancestors().nth(3).map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ofx-conformance.md")
}

#[test]
fn the_bench_describes_instances_and_renders_with_no_refused_suite_call() {
    let source = fixture_frame();
    let mut rows = Vec::new();

    // The tally is the assertion, so it starts at nought here and nothing else
    // in this binary runs beside it (one test, deliberately: the host's state
    // and its counters are process-wide).
    forget();

    let root = tempfile::tempdir().expect("a temporary directory");
    match a_fixture_bundle(root.path()) {
        Some(binary) => drive_bundle(&binary, &source, &mut rows),
        None => eprintln!(
            "the fixture bundle was skipped — {} was not built. cargo build -p lumit-ofx-testplug",
            test_plugin_file_name()
        ),
    }

    let (bench, looked_in) = bench_bundles();
    if bench.is_empty() {
        eprintln!(
            "the conformance bench was skipped — no OFX bundle in {}. Fetch and build \
             openfx-misc and ntsc-rs with `cargo run -p lumit-bench --bin ofx-bench`, or point \
             LUMIT_OFX_BENCH at a folder of bundles (docs/impl/ofx-host.md §5).",
            looked_in
                .iter()
                .map(|d| d.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    for binary in &bench {
        drive_bundle(binary, &source, &mut rows);
    }

    let (bad_handle, bad_value) = (answered(Status::ErrBadHandle), answered(Status::ErrValue));
    let table = table(&rows, bad_handle, bad_value);
    eprintln!("{table}");
    let out = out_path();
    if let Err(e) = std::fs::write(&out, &table) {
        eprintln!("the table could not be written to {}: {e}", out.display());
    }

    let failures: Vec<&Row> = rows
        .iter()
        .filter(|row| matches!(row.outcome, Outcome::Failed(_)))
        .collect();
    assert!(
        failures.is_empty(),
        "{} of {} plugin/context pairs failed:\n{table}",
        failures.len(),
        rows.len()
    );
    assert_eq!(
        bad_handle, 0,
        "the host had to answer kOfxStatErrBadHandle during the pass:\n{table}"
    );
    assert_eq!(
        bad_value, 0,
        "the host had to answer kOfxStatErrValue during the pass:\n{table}"
    );
    assert!(
        !rows.is_empty(),
        "nothing was driven at all — neither the fixture bundle nor a bench"
    );
}
