# Working on Lumit

Lumit is a native, Windows-first motion-graphics and compositing editor. After Effects'
compositing and Vegas' retiming in one application, built to stay responsive whatever the
layer and keyframe count. The engine is Rust on wgpu and the interface is Flutter. The two
meet at one crate. It's GPLv3 and specified docs-first: `docs/` says what the code does.

This file is for anyone who wants to change it. It assumes you know editing software and
can program, but not Rust, Dart or GPU work. It starts with building and running, then how
a change lands, then the rules, then the map of the code and the docs.

## 1. Build and run

### What you need

| Tool | Version | Why |
|---|---|---|
| Rust | 1.97.1, pinned by `rust-toolchain.toml` | rustup reads the file and fetches that version, you don't pick one |
| FFmpeg | 8.1, shared build | rsmpeg's bindings describe FFmpeg 8's structures. Against 7 or 9 the code compiles and reads the wrong offsets |
| LLVM | 18, for libclang | The binding generator reads FFmpeg's C headers through it. A much newer libclang emits blank structures |
| Flutter SDK | stable (CI uses 3.47.1) | On Windows you also need Visual Studio's C++ desktop workload, since the runner is a C++ program and Rust links with the same tools |

### Windows

1. Download `ffmpeg-n8.1-latest-win64-gpl-shared-8.1.zip` from the BtbN FFmpeg builds
   (github.com/BtbN/FFmpeg-Builds/releases) and unzip it under `%USERPROFILE%\ffmpeg\`.
   GPL because Lumit is GPL, shared because the app loads the DLLs at run time. A dated
   `autobuild-*` asset unzips to a folder the script's `ffmpeg-n8.1-*` search misses, so
   pass it with `-FfmpegDir <folder>` or set `KIRIKO_FFMPEG_DIR`.
2. `winget install LLVM.LLVM --version 18.1.8` and `winget install Rustlang.Rustup`.
3. From the repo root, once per terminal, or once for good with `-Persist`:

   ```powershell
   . .\scripts\win-dev-env.ps1 -Persist
   ```

   The leading dot applies the variables to your shell rather than one that closes. The
   script finds the FFmpeg folder and LLVM, sets `FFMPEG_LIBS_DIR`, `FFMPEG_INCLUDE_DIR`
   and `LIBCLANG_PATH`, and puts FFmpeg's `bin` on `PATH` for the DLLs and the `ffmpeg`
   command the tests use.
4. Test the engine, then run the app:

   ```powershell
   cargo test --workspace
   cd flutter_ui
   flutter run -d windows
   ```

If Flutter finds no supported devices, run `flutter config --enable-windows-desktop`
once. If the window opens and closes straight away, or the console names a missing DLL,
FFmpeg isn't on `PATH` in that terminal.

### macOS

Homebrew has no `ffmpeg@8` formula and its plain `ffmpeg` is already 9.x, which rsmpeg
doesn't support. The working recipe is CI's, `.github/actions/ffmpeg8-macos`. `brew
extract` lifts the 8.1.2 formula out of homebrew-core's history into a local tap, builds
it, exports `FFMPEG_PKG_CONFIG_PATH` and fails the build if the linked major isn't 8.
Follow that file. libclang comes with the developer tools. Then `cargo test --workspace`,
and `flutter run -d macos` from `flutter_ui/`.

CI tests the engine on macOS and compiles the app (`flutter build macos --release`).
`release.yml` makes the `.dmg` with `packaging/macos/make-dmg.sh`, which copies the FFmpeg
dylibs into the app and rewrites their install names.

### Linux

FFmpeg is found through `pkg-config`, so install the development packages, `pkg-config`
and `clang`. FFmpeg needs no environment variable.

```sh
# A distribution whose FFmpeg is 8.x (`pkg-config --modversion libavutil` prints 60.x)
sudo apt install pkg-config clang libavcodec-dev libavformat-dev libavutil-dev \
  libswscale-dev libswresample-dev libavfilter-dev libavdevice-dev

# Arch / Artix: the unversioned clang is LLVM 19+, so name 18
sudo pacman -S ffmpeg pkgconf clang18 llvm18
```

If your default `clang` is newer than 18, say which libclang to use, then test and run:

```sh
export LIBCLANG_PATH=/usr/lib/llvm18/lib          # Debian/Ubuntu: /usr/lib/llvm-18/lib
cargo test --workspace
cd flutter_ui && flutter run -d linux
```

A distribution still on FFmpeg 6 or 7 (Ubuntu 24.04 LTS, and every Debian and Ubuntu
release before FFmpeg 8 landed) needs a newer release, a self-built FFmpeg 8, or the route
CI takes: unpack the BtbN `linux64-gpl-shared-8.1` tarball `ci.yml` pins (a dated
`autobuild-*` release, since the rolling `latest` tag rotates its assets out), rewrite
`prefix=` in its `lib/pkgconfig/*.pc` files to where you unpacked it, and add that
folder to `PKG_CONFIG_PATH` and its `lib/` to `LD_LIBRARY_PATH`. `ci.yml` has the exact
commands. The Linux CI job installs lavapipe, a Vulkan driver that draws on the CPU, so
the GPU tests run with no card. Linux releases ship as a single-file Flatpak only.

### The commands you type

From the repo root (on Windows, after dot-sourcing `win-dev-env.ps1`):

```
cargo test --workspace                               # every engine test
cargo test -p lumit-core                             # one crate
cargo test -p lumit-core some_test_name              # one test, by part of its name
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo build -p lumit_bridge                          # the library the Flutter tests load
.\scripts\check.ps1                                  # fmt, clippy, tests, in CI's order
.\scripts\check.ps1 -Crate lumit-core                # the same, testing one crate
```

From `flutter_ui/`:

```
flutter run -d windows                # build and launch, add --release to judge speed
flutter test test/theme_test.dart     # one test file
flutter analyze                       # the lint pass, must stay clean
```

Three things to know before the first run:

- The crate folder is `crates/lumit-bridge`, the package is `lumit_bridge`. Cargo wants the
  package name, so `cargo build -p lumit-bridge` matches nothing.
- `flutter run` compiles the Rust side itself. `flutter_ui/rust_builder/` (vendored
  cargokit) builds `lumit_bridge`, and the Windows and Linux CMake files build
  `lumit-ofx-broker` and `lumit-aplug-broker` beside it (the macOS Xcode build doesn't).
  The first run takes minutes, later runs are quick. While it runs, `r` hot-reloads Dart,
  `R` restarts, `q` quits. A Rust change needs `q` and a fresh run, the library loads once.
- A debug build still optimises. The dev profile runs at `opt-level = 1`, its dependencies
  at 2, and `lumit-core`, `lumit-eval`, `lumit-gpu` and `lumit-render` at 3, because that's
  where the hot loops live and a lens flare bake ran 16 times slower without it. A debugger
  steps unevenly through those four crates, so use the tests.

## 2. Making a change

1. **Find the doc first.** The numbered specs in `docs/` say what the behaviour is, and
   `docs/impl/` says how the hard parts work. Read the matching impl note before
   implementing anything it covers. If your change disagrees with a doc, or needs an
   exception to a rule, the doc changes in the same commit.
2. **Make the change.** Keep the diff to what the change needs.
3. **Run the tests.** `.\scripts\check.ps1 -Crate lumit-core` while iterating, and
   `.\scripts\check.ps1` before a commit. The raw form is `cargo test --workspace`.
4. **Add the test.** New behaviour lands with a test that proves it. A bug fix lands with a
   regression test that fails without the fix.
5. **Commit with what and why.** A pull request states its spec reference and, if it
   added strings, lists the new keys (section 3).

### Testing notes

- **A test writes a file only when asked for by name.** After changing an effect's
  declaration, run `cargo test -p lumit-core regenerate_fx_reference -- --ignored` and
  commit the updated `crates/lumit-core/fx-reference.json`, or its guard test fails and
  tells you to.
- **Shaders are validated without a graphics card.**
  `crates/lumit-gpu/tests/wgsl_validates.rs` runs naga, the compiler wgpu uses, over every
  `.wgsl` kernel and fails on anything it would reject. That proves a shader is valid, not
  right. The CPU-oracle comparisons do that, and they need a card.
- **GPU tests share one device and one set of compiled engines** per test process. The
  first test to ask builds them, and every later test borrows the set in turn. The borrow
  is a lock, which keeps GPU tests serial while the CPU tests around them run in parallel.
  Engine state (flare bakes, compiled custom shaders) is wiped between borrowers. A test
  that needs its own device opens one. Borrowing the shared device twice on one thread
  fails with a message rather than deadlocking. `crates/lumit-gpu/src/test_support.rs` and
  `crates/lumit-render/src/headless.rs` hold it. No adapter means a skip, unless
  `LUMIT_REQUIRE_GPU=1` turns the skip into a failure, as the Linux CI job does.
- **Close every project you open, in tests too.** Each open project has one render worker
  holding a GPU connection, and closing is what hands it back. Leaked projects, one per
  test, once ran the CI machine out of memory.
- **The frb tests need the bridge library and FFmpeg on `PATH`.** `flutter_ui/test/frb/`
  loads `target/debug/lumit_bridge.dll` and drives the real engine. Build it first with
  `cargo build -p lumit_bridge`. A stale library fails a content-hash check in every test's
  setup, which shows up as "found 0 widgets" over and over. The library loads the FFmpeg
  DLLs, so make sure FFmpeg's `bin` is on `PATH` in that shell, or the load fails with
  error code 126 naming the bridge rather than the DLL it couldn't find.
- **Run single Flutter test files while iterating**, from `flutter_ui/`:
  `flutter test test/frb/timeline_panel_frb_test.dart`. A bare `flutter test` starts a
  process per file in parallel, and each frb file opens the engine and a graphics device.
  CI runs the whole suite. Locally, name the files you touched.
- **Nothing personal or machine-specific goes in a committed file.** The repo is public.
  Paths under your user folder, your monitor layout, your test footage: none of it.

## 3. The rules

### Code

`docs/14-ENGINEERING-RULES.md` is binding on every line. The ones you meet first:

- **Time is an exact rational.** `SourceTime`, `ClipTime`, `LayerTime` and `CompTime` are
  distinct types, so mixing them doesn't compile. Conversions are named functions on the
  objects that own them. Authoritative time is never `f32` or `f64`. Floats appear only at
  leaves (display, slider scratch values, numeric kernels) and convert back before storage
  or comparison. Frame rates are rational (`30000/1001`, never `29.97`). Time rounds to a
  frame in one function per direction, `FrameRate::frame_at` and `FrameRate::time_of_frame`.
- **No panics in engine crates.** The workspace lints deny `unwrap_used`, `expect_used`,
  `panic`, `todo` and `unimplemented`, and `unsafe_code`. The FFI crates (the OFX and
  audio-plugin hosts, their brokers and test plugins, and the bridge) allow `unsafe_code`
  in their own `Cargo.toml`, nothing else may. Every fallible boundary returns a typed
  error (`thiserror` enums per crate). GPU device loss is an enum variant that triggers
  recovery. A failed effect, expression or plugin draws as an errored placeholder and the
  frame carries on. Tests and build scripts may panic. The bridge's `#[frb]` functions are
  outside clippy's reach, so a CI grep enforces the rule on `src/api/`.
- **Threads have fixed roles.** The UI thread edits the document and paints. It never
  evaluates a node, decodes a frame, runs an expression, does blocking IO or waits on a
  frame. Workers read the snapshot their job was made with, never the live document.
- **No lock is held across** an `.await`, a GPU submit or readback wait, a channel send
  that can block, an FFI call into FFmpeg, OFX or CUDA, or plugin IPC. Lock scope is a
  block you can read at a glance. A new `Mutex` or `RwLock` in a `lumit-eval`,
  `lumit-core` or `lumit-cache` hot path needs review naming who holds it, for how long,
  and why a channel won't do.
- **Allocations are budgeted.** Frame-sized buffers come from the pools in `lumit-gpu` and
  `lumit-media`, which account against the resource governor. A frame-sized
  `Vec::with_capacity` outside them is a review reject. Every channel between threads is
  bounded, and the sender blocks, drops or degrades by a policy chosen at the call site.
- **Cancellation everywhere.** Every loop over frames, rows, tiles, clips or nodes checks
  its epoch each iteration and returns `Err(Cancelled)`, which schedulers treat as a clean
  stop. Anything that can take more than about 100 ms is cancellable and reports progress.
- **Determinism.** Same project, same inputs, same pixels on export, every run. No wall
  clock, no thread ids, no hash-map iteration order in evaluation. Randomness is seeded
  from the node, property, time and user seed. Float reductions use a fixed association
  order. Degradation, proxies and preview resolution touch preview only. Export always
  evaluates at full quality.

`docs/01-GLOSSARY.md` binds identifiers, comments, UI strings, commits and docs. The words
that catch people: **layer** not track, **speed** not velocity, **Retime** not time remap,
**export** not render for user-facing output, and **clip** only inside a Sequence layer.
Tracking a feature through a shot is the trade's verb and stays. A new concept gets its
glossary entry first, then the identifier follows it.

### Tests and CI

Every feature lands with its tests, and every bug fix lands with a regression test that
fails without it. An effect that touches pixels ships a CPU oracle and a golden frame the
GPU kernel is checked against. Time and Retime get property tests. Long-running work gets
a test that cancels it. Anything that deserialises or crosses IPC adds a fuzz corpus
entry, once the corpus exists (the rules note the fuzz targets aren't set up yet).

Performance budgets gate merges. `docs/13-PERFORMANCE-RULES.md` names reference hardware
and numbers against it: UI frame time under 8 ms during any interaction, a scrub showing
its first frame within 50 ms, warm-cache playback at 60 fps with no drops. Two rules serve
all of them: the interface never waits for the engine, and the engine degrades rather than
crashes. Panels that list the document (the Timeline, the project panel, the Graph editor)
draw only what's on screen. A widget that iterates every layer or keyframe fails the stress
budget. `lumit-bench` measures the budgets on CI and fails a run 1.6 times worse than the
checked-in baseline for that runner (`crates/lumit-bench/baselines/`, recorded at a tenth
of the work area, `BENCH_SPAN_FRACTION=10`, and a baseline only compares at its own
fraction). Absolute numbers are asserted only under `LUMIT_REFERENCE_HW=1`. A performance
or never-crash claim added to a doc lands with the CI check that proves it.

A red CI blocks everything else. `.github/workflows/ci.yml` runs on every pull request
and every push to `main` (and by hand from the Actions tab):

| Job | Checks |
|---|---|
| `check` (macOS) | `cargo fmt --all -- --check`, clippy with `-D warnings`, the workspace tests, a release-mode compile |
| `windows` | The shipping target: clippy and the workspace tests with media, GPU tests on WARP |
| `linux` | The same on lavapipe with `LUMIT_REQUIRE_GPU=1`, and the bridge with its media feature off |
| `performance` | `lumit-bench` against the runner's baseline |
| `flutter-linux` | `flutter analyze`, the bridge build, the whole Flutter suite (with `LUMIT_NO_ZERO_COPY_VIEWER=1`), clippy on `lumit-gpu` with `shared-texture-linux`, `flutter build linux --release` |
| `flutter-macos` | `flutter build macos --release` |
| `codegen-fresh` | Regenerates the bridge and fails on any diff |
| `coverage` | `cargo llvm-cov` over the FFmpeg-free engine crates, `--fail-under-lines 80`, and the threshold only rises |
| `no-hex-outside-theme` | Colour literals in `flutter_ui/lib` outside `theme/`, hex in `crates/`, and `check-icon.py` |
| `cargo-deny` | Licences, advisories and sources, per `deny.toml` |
| `no-panics-in-frb-api` | `.unwrap()`, `.expect(`, `panic!`, `todo!` or `unimplemented!` in the shipping half of `crates/lumit-bridge/src/api.rs` and `src/api/**` (test modules and `*tests.rs` files exempt) |
| `ofx-conformance` | The OFX host describing, instancing and rendering every plugin in openfx-misc and ntsc-rs |
| `ofx-handle-fuzz` | `lumit-ofx`'s `handle_fuzz` under AddressSanitizer, on the nightly compiler |

The definition of done in `docs/14-ENGINEERING-RULES.md` section 10 is the checklist a
pull request answers: spec reference, tests, cancellation and progress, budget compliance,
error paths, glossary, determinism, regression coverage.

### Strings and translations

Every user-facing string lives in `flutter_ui/lib/l10n/app_en.arb` and is read as
`l10n.someKey`, never written inline in a widget. A new or reworded string lands in the
arb in the same commit, with its `@key` description saying where the string appears, since
that's the only context a translator gets. `flutter pub get` regenerates `l10n/gen/`.

Two things are easy to miss:

- **A string the engine can send needs two entries.** Keymap action descriptions, effect
  and category labels, and anything else Rust hands over as English text need the
  `app_en.arb` key and a matching entry in `flutter_ui/lib/l10n/engine_labels.dart`.
  `flutter_ui/test/l10n/engine_labels_test.dart` walks the engine's own tables and fails on
  any label with no entry. A keymap action is a string.
- **The other `app_*.arb` files are never hand-edited.** `app_de`, `app_es`, `app_kk`, `app_pl`,
  `app_uk`, `app_zh` and `app_zh_Hant` are written by `scripts/translations.ps1` from what the
  translation page on lumitlab.com sends back, so a hand edit is overwritten by the next
  run. A new English key leaves the other languages short, which is expected and falls
  back to English. List the new keys in the commit message and the pull request so they
  reach the translation page. A string nobody was told about stays English for a release.

### Design

`docs/15-DESIGN.md` is the design language: dark-first, semantic colour tokens only, mono
type for every number, hairline borders rather than shadows, one accent, no UI that scolds.
Every colour comes from the theme struct in `flutter_ui/lib/theme/`. A hex literal, a
Material `Colors.*` or a `Color.fromARGB` from number literals anywhere else in
`flutter_ui/lib` fails CI. Read the colour off `LumitTheme`, and add a token if none fits.
Lumit's own widgets over stock Material ones.

Voice, in the interface and the docs: British English, sentence case, calm, no exclamation
marks, no emoji.

### The frontend

The Flutter app is a view. Opening files, decoding, compositing, caching, mixing and
export are the engine's. Dart displays values and forwards calls, and when something has
to be decided, Rust decides it. Editing logic in `flutter_ui/` will drift from the engine,
so it belongs in Rust.

`docs/17-BRIDGE-CONTRACT.md` is the single source of truth for the seam. Read it before
touching either side. In short, Dart never holds a copy of the document. It holds handles
and calls methods on them. Rust pushes a small "this changed" message down a stream so
only the affected part of the interface redraws. Video frames cross as raw pixel buffers
or, on the fast path, as GPU memory shared without a copy. The functions Dart can call are
the same in every build. A feature flag changes what one does, never whether it exists.

`crates/lumit-bridge/src/api/**` declares in Rust everything Dart may call. A generator
writes both halves from it: `crates/lumit-bridge/src/frb_generated.rs` and
`flutter_ui/lib/src/rust/**`. Neither is ever hand-edited, and the `codegen-fresh` job
regenerates and diffs. After editing `api/**`:

```
cd flutter_ui
flutter pub get
flutter_rust_bridge_codegen generate
cd ..
cargo build -p lumit_bridge
```

or `.\scripts\codegen.ps1`, which sets the Windows environment, runs those four commands
and ends with `git status --short` so you see what changed (`-SkipBuild` stops before the
cargo build). The generator runs from `flutter_ui/` because `flutter_rust_bridge.yaml`
lives there, and it must be the pinned version:
`cargo install flutter_rust_bridge_codegen --version 2.12.0 --locked`. The last step is
the one people forget, since the Flutter tests load the library it builds.

Where things live in `flutter_ui/lib`:

| Path | What it is |
|---|---|
| `main.dart` | Entry point |
| `shell/` | The app shell: menu bar, tool bar, dock, status line, dialogues, settings window, command palette, splash, startup failure |
| `panels/` | One file per panel or panel piece: Viewer, Timeline, Graph editor, effect controls, project, scopes, audio |
| `state/` | App state and Dart-side caches: `app_state.dart` (`LumitState`), `ui_state.dart`, `comp_model.dart` (what the panels read), `comp_time.dart`, `settings.dart`, `faults.dart` |
| `widgets/` | Shared controls: colour picker, curve editor, marquee, time readout, `controls/` |
| `builder/` | `ProjectItemBuilder`: a widget that rebuilds when one project item changes |
| `data/` | The expression function reference, parsed from the engine's JSON |
| `probe/` | The parked UI performance probe behind `docs/impl/ui-performance.md`, compiled in only with `--dart-define=LUMIT_PROBE_PROJECT` |
| `theme/` | Every colour and type token, and the only place a colour is spelled out |
| `l10n/` | `app_en.arb`, `engine_labels.dart`, `strings.dart`, the generated `gen/`, the ingested translations and `translation-state.json` |
| `icons/` | Lumit's own icon set. `lumit_icons.dart` is generated from `tool/icons/glyphs.json` by `dart run tool/icons/gen_lumit_icons.dart` (from `flutter_ui/`) |
| `src/rust/` | Generated bindings, never edited |

The `_frb` suffix on a file means it calls the bridge. Window size, placement and the
10-second show-anyway fallback (a start that never paints still gets a window to close)
live in the platform runners under `flutter_ui/windows`, `macos` and `linux`, not in Dart.

## 4. The map

### Crates

The engine is one Cargo workspace, and every folder matching `crates/lumit-*` is a member.
One job per crate.

| Crate | Does |
|---|---|
| `lumit-core` | Rational time, the document, operations and undo, the snapshot store, expressions. The root, it depends on nothing above it |
| `lumit-project` | The `.lum` container, the operation journal, autosave and crash recovery |
| `lumit-eval` | The evaluator: content-hash frame keys, the graph compiler, epochs, the worker pool, the scheduler core |
| `lumit-render` | The pixel pass: the decode worker, draw lists, the compositor, effect dispatch, cache tiers, export, the headless renderer |
| `lumit-gpu` | The one wgpu device, the WGSL effect kernels, the compositor, the colour engine, readback |
| `lumit-cache` | The frame cache (Nebula): byte-budgeted RAM and disk tiers |
| `lumit-flow` | Optical flow (DIS): a CPU oracle and its WGSL twin, for Retime interpolation and flow motion blur |
| `lumit-media` | FFmpeg through rsmpeg: probing, the frame index, exact seeking, hardware decode, encode |
| `lumit-audio` | Playback and the audio clock (Pulsar): cpal output, mixing, waveforms, beat detection |
| `lumit-text` | Text rasterisation |
| `lumit-colour` | OCIO colour management, implemented natively: config parsing, transforms, the deterministic bake |
| `lumit-track` | The 2D tracking substrate: feature detection, affine KLT tracks, planar tracks, the camera solve and bundle adjustment |
| `lumit-roto` | The roto brush's arithmetic: seeding, geodesic segmentation, the edge refine, carrying a matte along a flow field |
| `lumit-import` | After Effects import: reading a capture bundle into a document, with a report of what changed |
| `lumit-keymap` | Shortcuts: chords, contexts, bindings, clash detection, no windowing code |
| `lumit-fx-macros` | `#[derive(Effect)]`: one declaration per built-in effect produces its catalogue entry, parameters and dispatch |
| `lumit-ofx` | The OpenFX host: suites, property sets, action dispatch, the out-of-process transport |
| `lumit-ofx-broker` | The expendable program a third-party OFX plugin runs in, one per bundle, so a crash costs one frame |
| `lumit-ofx-testplug` | Minimal OFX plugins written here, so the host's tests need no download |
| `lumit-aplug` | The audio plugin host: CLAP first, then VST3 |
| `lumit-aplug-broker` | The program an audio plugin runs in, and a crash plays one block dry |
| `lumit-aplug-testplug` | Minimal CLAP and VST3 plugins for the host's tests: one library exporting both entry points, laid out either way |
| `lumit-bench` | The headless performance harness the CI gates run, plus the `ofx-bench` binary. Nothing in the application depends on it |
| `lumit-bridge` | The seam: a `cdylib` (and a `staticlib` for the macOS link) whose `api` module is the whole surface Flutter calls. Package name `lumit_bridge` |

Nova is `lumit-eval` and `lumit-gpu` together: the render pipeline.

**The one dependency rule:** dependencies point downward only. `lumit-bridge` depends on
the engine crates, and the engine crates depend on `lumit-core`. No engine crate depends
on the bridge or on any UI, so the interface can be replaced without opening the engine.
`lumit-core` depends on no GPU, codec or audio library, so the document tests on any
machine. A circular dependency is a build error, so the piece two crates share moves down.

### Outside `crates/`

| Path | What it is |
|---|---|
| `flutter_ui/` | The Flutter app (section 3). Builds the bridge itself through `rust_builder/` |
| `web/` | lumitlab.com: an Astro site |
| `web-docs/` | docs.lumitlab.com, the user manual: Astro Starlight |
| `scripts/` | The routines as PowerShell: `build.ps1`, `check.ps1`, `codegen.ps1`, `translations.ps1`, `manual-pages.ps1`, `shots.ps1`, `win-dev-env.ps1`. The ones that shell out (`build`, `check`, `codegen`, `manual-pages`, `shots`) print every command before running it, and every script answers `-?`. Beside them `gen-icons.py` (renders the brand SVGs in `assets/brand/` to the `.ico` and PNG rasters), `check-icon.py` and `discord-release.mjs` |
| `packaging/` | `windows/` (Inno Setup), `macos/` (`make-dmg.sh`), `linux/` (desktop entry, MIME types), `flatpak/` (the manifest) |
| `tools/` | `ae-bridge`, the After Effects script that writes an import bundle, and `ae-audit`, its audit kit |
| `assets/` | Brand SVGs, the embedded Inter font, the audio click |
| `.github/workflows/` | `ci.yml` on pull requests and pushes to `main`, and `release.yml` on a `v*` tag |
| Root files | `rust-toolchain.toml` pins Rust, and `deny.toml` is the licence and advisory policy |

The two sites are outside the Cargo workspace and nothing depends on them. Cloudflare
builds them from `main` on push. `npm install` once, then `npm run dev` to preview.

### How the engine runs

Threads have fixed roles (`docs/05-ARCHITECTURE.md` section 2):

| Thread | Does |
|---|---|
| UI | Takes input, applies edits, publishes snapshots and paints. Never heavy work |
| Worker pool | `cores - 3` threads, at least 2. Evaluation jobs come in two classes, interactive and background, and interactive pre-empts at job boundaries |
| Decode | One per active media stream, never on the pool, because a long-GOP seek would stall it |
| IO | The disk cache, the journal and export files |
| Analysis | Camera tracking, one at a time, since it's a decode that runs for minutes |
| Audio pair | The cpal callback, which only reads a lock-free ring buffer, and an audio-render thread that fills it ahead of time |
| GPU-submit | The only thread that submits to the wgpu queue |

Three mechanisms make this safe, and you meet them by name:

- **Snapshots.** The document lives behind an `ArcSwap` in `lumit-core`'s store. An edit
  produces a new immutable document and swaps the pointer. A worker mid-job keeps the old
  one and new work sees the new one. Nobody reads a half-finished edit.
- **Epochs.** Every frame request carries a generation number per consumer: one for the
  Viewer, one per export, one for background warming. Moving the playhead bumps the
  Viewer's. Jobs check theirs at node boundaries and between tiles and return
  `Err(Cancelled)` when stale. Nothing is killed, and a stale frame that finishes still
  enters the cache. `docs/impl/playback-scheduler.md` has the detail.
- **Bounded channels.** Threads hand each other work through queues of fixed length. A
  full queue makes the sender wait. That back-pressure is how the app degrades under load.

Playback is decode ahead, evaluate, present over those queues. The audio clock is master:
the presented video frame is a function of the samples the audio callback has consumed. If
evaluation falls behind, frames drop and preview degrades. Audio never waits.

Rust words you'll meet in the code, with the fuller course in `docs/learn/RUST.md`:

| Word | Meaning |
|---|---|
| `Result<T, E>` | Either `Ok(value)` or `Err(why)`, and `?` passes the error up. An error is a value, not a crash |
| `Option<T>` | `Some(value)` or `None`. There's no null |
| `struct` / `enum` | A record / a choice between shapes, and `match` must handle every case |
| `trait` | A named set of functions a type promises. A function can accept any type that has them |
| `Arc<T>` | A shared, reference-counted handle, freed when the last holder drops it |
| `&x` / `&mut x` | Borrow read-only / borrow with permission to change |
| `async` | Not used in the engine. Threads and channels instead, on purpose |

## 5. The docs

The numbered specs in `docs/` are canonical. When code and a spec disagree the spec wins,
and a doc changes in the same commit as the code. `docs/README.md` is the index and
explains the three kinds of document: durable specs, living state such as `TODO.md`, and
frozen history under `archive/`. Specs end with an Open questions list. Resolving one
means editing the spec, in the same commit as the code that settles it.

| Question | Doc |
|---|---|
| Why Lumit exists, and what it refuses to be | `00-VISION.md` |
| What a thing is called | `01-GLOSSARY.md`, section 9 is the banned list |
| What a project, comp, layer, clip, property or keyframe is | `03-DATA-MODEL.md` |
| How Retime works, and the two graph lenses | `04-RETIMING.md` |
| Crates, threads, snapshots, the evaluation graph, the GPU | `05-ARCHITECTURE.md` |
| Render order, colour, caching, preview, export | `06-RENDER-PIPELINE.md` |
| Panels, workspaces, the Viewer, the Timeline, the keymap | `07-UI-SPEC.md` |
| The built-in effects | `08-EFFECTS.md` |
| Audio: the sync toolkit and the future Composer | `09-AUDIO.md` |
| The `.lum` file, sidecar caches, autosave | `10-FILE-FORMAT.md` |
| After Effects import and the fidelity matrix | `11-AE-IMPORT.md` |
| OFX hosting, the LFX native API, expressions | `12-PLUGINS.md` |
| Budgets, the resource governor, the degradation ladder | `13-PERFORMANCE-RULES.md` |
| The rules every line of code follows | `14-ENGINEERING-RULES.md` |
| Colour, type, density, motion, voice | `15-DESIGN.md` |
| Phases and their gates | `16-ROADMAP.md` |
| How the frontend and the engine talk | `17-BRIDGE-CONTRACT.md` |

Around them:

| Path | What it is |
|---|---|
| `docs/impl/` | The authoritative how for each hard topic: the algorithm, the data layout, the traps and the test plan. Read the matching note before implementing what it covers, and implement its test plan with the feature. Where a note and a spec conflict, the spec wins and the note is fixed with it. `docs/impl/README.md` is the table |
| `docs/TODO.md` | The backlog, in Now, Next and Later. Delete an item when it lands, its regression test is the record |
| `docs/learn/` | The codebase as built, with real excerpts. `00-MAP.md` is the system on one page and where to change what, `01` to `06` the areas of the repo, `07-BUILD-SHIP.md` the build, `08-WEBSITES.md` the two sites, `09-DOING-IT-YOURSELF.md` the runbook with every routine command and how it fails, `10` a feature end to end, and `RUST.md`, `FLUTTER.md`, `WGSL.md` the languages from Lumit's own code. Start at `docs/learn/README.md` |
| `docs/research/` | The background research the specs came from. Not canonical |
| `docs/archive/` | Frozen, dated history. Read it, never update it |
| The user manual | docs.lumitlab.com, built from `web-docs/src/content/docs/`. What the application does from the user's side. Its effect pages are generated from the engine's own catalogue by `scripts/manual-pages.ps1` |
