//! The Sort sample extension's plugin, built from its C source, found in its
//! bundle as an installed extension's is, and drawn through a broker.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

use lumit_ofx::bundle::{scan_dir, BUNDLE_ARCH_DIR};
use lumit_ofx::describe::Context;
use lumit_ofx::image::Frame16;
use lumit_ofx::instance::ParamSnapshot;
use lumit_ofx::ipc::broker::{Broker, BrokerConfig};
use lumit_ofx::render::RenderRequest;

const SAMPLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/extensions/sort"
);

/// Compile sort.c into `<extension>/Sort.ofx.bundle`, as its own comments say
/// to, and answer with the extension's folder.
fn build_sort(root: &Path) -> PathBuf {
    let extension = root.join("sort");
    let binaries = extension
        .join("Sort.ofx.bundle")
        .join("Contents")
        .join(BUNDLE_ARCH_DIR);
    std::fs::create_dir_all(&binaries).unwrap();
    let out = binaries.join("Sort.ofx");
    let triple = if cfg!(windows) {
        format!("{}-pc-windows-msvc", std::env::consts::ARCH)
    } else if cfg!(target_os = "macos") {
        format!("{}-apple-darwin", std::env::consts::ARCH)
    } else {
        format!("{}-unknown-linux-gnu", std::env::consts::ARCH)
    };
    let compiler = cc::Build::new()
        .target(&triple)
        .host(&triple)
        .opt_level(2)
        .cargo_metadata(false)
        .cargo_warnings(false)
        .try_get_compiler()
        .expect("a C compiler");
    let mut command: Command = compiler.to_command();
    // cl leaves its object file in the folder it runs in.
    command
        .current_dir(root)
        .arg(Path::new(SAMPLE).join("sort.c"));
    if compiler.is_like_msvc() {
        command
            .arg("/LD")
            .arg(format!("/Fe:{}", out.display()))
            .args(["/link", "/NOIMPLIB", "/NOEXP"]);
    } else if cfg!(target_os = "macos") {
        command.arg("-bundle").arg("-o").arg(&out);
    } else {
        command.args(["-shared", "-fPIC", "-o"]).arg(&out);
    }
    let status = command.status().expect("the compiler runs");
    assert!(status.success(), "sort.c did not build");
    std::fs::copy(
        Path::new(SAMPLE).join("extension.json"),
        extension.join("extension.json"),
    )
    .unwrap();
    extension
}

/// Each row's runs of pixels brighter than the threshold, darkest first.
fn sorted_rows(pixels: &[[f32; 4]], width: usize, threshold: f32) -> Vec<[f32; 4]> {
    let mut out = pixels.to_vec();
    for row in out.chunks_mut(width) {
        let bright = |p: &[f32; 4]| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2];
        for run in row.split_mut(|p| bright(p) <= threshold) {
            run.sort_by(|a, b| bright(a).total_cmp(&bright(b)));
        }
    }
    out
}

#[test]
fn the_sort_sample_sorts_bright_runs_along_rows() {
    let root = tempfile::tempdir().unwrap();
    let extension = build_sort(root.path());

    // The bundle folder the manifest names is what the plugin scan is given.
    let found = scan_dir(&extension.join("Sort.ofx.bundle"));
    assert_eq!(found.len(), 1, "{found:?}");
    let mut config = BrokerConfig::new(&found[0], (8, 4));
    config.exe = Some(PathBuf::from(env!("CARGO_BIN_EXE_lumit-ofx-broker")));
    let mut broker = Broker::spawn(config).unwrap();
    let described = broker.describe().unwrap();
    let plugin = described
        .iter()
        .position(|d| d.identifier == "com.lumitlab.sort")
        .expect("the Sort plugin");
    let params = ParamSnapshot::from_defaults(&described[plugin]);
    let instance = broker
        .create_instance(plugin as u32, Context::Filter, params)
        .unwrap();

    // Greys, each row a shuffle of eight levels, none near the threshold.
    let (width, height) = (8, 4);
    let source: Vec<f32> = (0..width * height)
        .flat_map(|i| {
            let v = ((i % width) * 5 + (i / width) * 3) % 8;
            let v = v as f32 / 8.0 + 0.03;
            [v, v, v, 1.0]
        })
        .collect();
    let source = Frame16::from_f32(width, height, &source).unwrap();
    let rendered = broker
        .render(
            instance,
            &RenderRequest::filter(0.0, source.clone()),
            &|_, _| None,
        )
        .unwrap();
    assert!(!rendered.errored, "{:?}", rendered.error);

    let read = |frame: &Frame16| -> Vec<[f32; 4]> {
        (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .map(|(x, y)| frame.pixel(x, y))
            .collect()
    };
    let input = read(&source);
    let expected = sorted_rows(&input, width, 0.25);
    assert_ne!(expected, input, "the frame has runs to sort");
    assert_eq!(read(&rendered.frame), expected);
}
