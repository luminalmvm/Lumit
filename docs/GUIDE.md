# Working on Lumit

Lumit is a Windows-first motion-graphics and compositing editor. The engine is Rust on
wgpu, the interface is Flutter, and the two meet at one crate, `lumit-bridge`. GPLv3.

The other docs here:

| Doc | What it holds |
|---|---|
| [01-GLOSSARY.md](01-GLOSSARY.md) | What things are called. Binding on code, UI text and commits |
| [05-ARCHITECTURE.md](05-ARCHITECTURE.md) | Threads, snapshots, the evaluation graph, the GPU |
| [13-PERFORMANCE-RULES.md](13-PERFORMANCE-RULES.md) | Budgets, the resource governor, the degradation ladder |
| [14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md) | The rules every line of code follows |
| [17-BRIDGE-CONTRACT.md](17-BRIDGE-CONTRACT.md) | How the frontend and the engine talk |

The user manual is docs.lumitlab.com, built from `web-docs/`.

## 1. Build and run

| Tool | Version | Why |
|---|---|---|
| Rust | pinned by `rust-toolchain.toml` | rustup reads the file, you don't pick one |
| FFmpeg | 8.1, shared build | rsmpeg's bindings describe FFmpeg 8. Against 7 or 9 it compiles and reads the wrong offsets |
| LLVM | 18 | The binding generator reads FFmpeg's headers through libclang. Newer ones emit blank structures |
| Flutter | stable (CI pins the version) | On Windows you also need Visual Studio's C++ desktop workload |

### Windows

1. Unzip `ffmpeg-n8.1-latest-win64-gpl-shared-8.1.zip` from the BtbN FFmpeg builds under
   `%USERPROFILE%\ffmpeg\`. A dated `autobuild-*` asset needs `-FfmpegDir <folder>`.
2. `winget install LLVM.LLVM --version 18.1.8` and `winget install Rustlang.Rustup`.
3. From the repo root: `. .\scripts\win-dev-env.ps1 -Persist`. The leading dot matters.
4. `cargo test --workspace`, then `cd flutter_ui; flutter run -d windows`.

If the window opens and closes straight away, FFmpeg's `bin` isn't on `PATH` in that
terminal.

### macOS

Homebrew has no `ffmpeg@8` and its `ffmpeg` is 9.x. Follow CI's recipe in
`.github/actions/ffmpeg8-macos`. Then `cargo test --workspace`, and `flutter run -d macos`
from `flutter_ui/`.

### Linux

Install the FFmpeg 8 dev packages, `pkg-config` and `clang` (on Arch, `clang18 llvm18`).
If your default clang is newer than 18, set `LIBCLANG_PATH` to LLVM 18's `lib`. A distro
on FFmpeg 6 or 7 needs the BtbN tarball the way `ci.yml` does it.

### Commands

From the repo root:

```
cargo test --workspace                      # every engine test
cargo test -p lumit-core some_test_name     # one test
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo build -p lumit_bridge                 # the library the Flutter tests load
.\scripts\check.ps1                         # what CI runs, in CI's order
.\scripts\check.ps1 -Crate lumit-core       # the same, one crate
```

From `flutter_ui/`:

```
flutter run -d windows                      # add --release to judge speed
flutter test test/theme_test.dart           # one file at a time, never the whole suite
flutter analyze
```

- The crate folder is `crates/lumit-bridge`, the package is `lumit_bridge`. Cargo wants the
  package name.
- `flutter run` builds the Rust side itself. A Rust change needs `q` and a fresh run.
- The dev profile optimises (`opt-level` 1, deps 2, the four hot crates 3), so a
  debugger steps unevenly. Use the tests.

## 2. Making a change

1. Keep the diff to what the change needs.
2. Only add a test when it's essential, such as one that guards lost work, a crash, or
   wrong pixels. When behaviour changes, update the test that covers it rather than
   adding another.
3. `.\scripts\check.ps1` before a commit.
4. If the change breaks a rule in these docs, change the doc in the same commit.

### Testing notes

- After changing an effect's declaration, run
  `cargo test -p lumit-core regenerate_fx_reference -- --ignored` and commit the updated
  `fx-reference.json`.
- `crates/lumit-gpu/tests/wgsl_validates.rs` checks every shader compiles. The CPU-oracle
  tests check they're right, and those need a GPU.
- GPU tests share one device per process and run one at a time. No adapter means a skip,
  unless `LUMIT_REQUIRE_GPU=1`.
- Close every project you open, in tests too. Each holds a render worker and a GPU
  connection.
- The frb tests in `flutter_ui/test/frb/` load `target/debug/lumit_bridge.dll`. Build it
  first, with FFmpeg on `PATH`. A stale library shows up as "found 0 widgets".
- Nothing personal or machine-specific in a committed file. The repo is public.

### CI

`.github/workflows/ci.yml` runs on every pull request and push to `main`. A red CI blocks
everything else.

| Job | Checks |
|---|---|
| `check` (macOS) | fmt, clippy `-D warnings`, the tests, a release compile |
| `windows` | clippy and the tests with media, GPU tests on WARP |
| `linux` | the same on lavapipe with `LUMIT_REQUIRE_GPU=1` |
| `performance` | `lumit-bench` against the runner's baseline, fails at 1.6x worse |
| `flutter-linux` | analyze, the whole Flutter suite, a release build |
| `flutter-macos` | a release build |
| `codegen-fresh` | regenerates the bridge and fails on any diff |
| `coverage` | engine line coverage, `--fail-under-lines 80`, only ever rises |
| `no-hex-outside-theme` | colour literals outside `flutter_ui/lib/theme/` |
| `cargo-deny` | licences, advisories, sources |
| `no-panics-in-frb-api` | `unwrap`, `expect`, `panic!` and friends in `lumit-bridge/src/api` |
| `ofx-conformance`, `ofx-handle-fuzz` | the OFX host against real plugins, and under ASan |

## 3. Strings, design and the frontend

### Strings

- Every user-facing string lives in `flutter_ui/lib/l10n/app_en.arb` with an `@key`
  description, and is read as `l10n.key`. Never inline.
- A string the engine sends (effect labels, keymap actions) also needs an entry in
  `flutter_ui/lib/l10n/engine_labels.dart`. `engine_labels_test.dart` fails without it.
- Never hand-edit the other `app_*.arb` files. `scripts/translations.ps1` writes them from
  the translation page on lumitlab.com.

### Design

Dark-first. Every colour comes from `LumitTheme` in `flutter_ui/lib/theme/`. A hex literal,
`Colors.*` or `Color.fromARGB` anywhere else fails CI, so add a token instead. Row heights
come from `DensityTokens`, corners and insets from `ShapeTokens`. Mono type for numbers,
hairlines not shadows, one accent. Lumit's own widgets over Material ones.

Voice: British English, sentence case, calm. No exclamation marks, no emoji.

### The frontend

Flutter is a view. It displays values and forwards calls. When something has to be
decided, Rust decides it. Read [17-BRIDGE-CONTRACT.md](17-BRIDGE-CONTRACT.md) before
touching either side.

After editing `crates/lumit-bridge/src/api/**`, run `.\scripts\codegen.ps1`. The generated
`frb_generated.rs` and `flutter_ui/lib/src/rust/**` are never hand-edited.

| `flutter_ui/lib/` | What it is |
|---|---|
| `shell/` | Menu bar, tool bar, dock, status line, dialogues, settings, command palette |
| `panels/` | One file per panel: Viewer, Timeline, Graph editor, effect controls, project, scopes, audio |
| `state/` | App state and Dart-side caches |
| `widgets/` | Shared controls |
| `theme/` | The only place a colour is spelled out |
| `l10n/` | Strings and translations |
| `icons/` | Lumit's icon set, generated from `tool/icons/glyphs.json` |
| `src/rust/` | Generated bindings |

A file ending `_frb` calls the bridge.

## 4. The map

One Cargo workspace. Every `crates/lumit-*` folder is a member, one job each.

| Crate | Does |
|---|---|
| `lumit-core` | Rational time, the document, ops and undo, the snapshot store, expressions, the effect declarations |
| `lumit-project` | The `.lum` file and the footage packed into it, the op journal, autosave, crash recovery |
| `lumit-eval` | Frame keys, the graph compiler, epochs, the worker pool, the scheduler core |
| `lumit-render` | The pixel pass: decode worker, draw lists, compositor, effect dispatch, cache tiers, export, the headless renderer |
| `lumit-gpu` | The one wgpu device, the WGSL kernels, the compositor, colour, readback |
| `lumit-cache` | The frame cache: RAM and disk tiers with byte budgets |
| `lumit-budget` | The resource governor's ledger |
| `lumit-flow` | Optical flow, CPU and WGSL |
| `lumit-media` | FFmpeg: probe, index, seek, hardware decode, encode. Reads PSD and Illustrator files by layer |
| `lumit-audio` | Playback, the audio clock, mixing, waveforms, beat detection |
| `lumit-text` | Text: system fonts, shaping, layout and rasterisation |
| `lumit-colour` | OCIO, implemented natively |
| `lumit-track` | Tracking and the camera solve |
| `lumit-roto` | The roto brush's maths |
| `lumit-ml` | Installed addons: model packs and the model runtime |
| `lumit-import` | After Effects import |
| `lumit-keymap` | Shortcuts |
| `lumit-ingress` | Limits for reading untrusted input |
| `lumit-peer` | Authenticating the plugin broker pipes |
| `lumit-share` | Shared projects: the host's end, a guest's end, and the encrypted channel between them |
| `lumit-relay` | The relay a shared project can meet at, and what both ends say to one. Also a program, `lumit-relay`, with nothing but std in it |
| `lumit-extensions` | Installed extensions: reading an `extension.json`, and the folder they live in |
| `lumit-fx-macros` | `#[derive(Effect)]` |
| `lumit-ofx`, `-ofx-broker`, `-ofx-testplug` | The OFX host, the process a plugin runs in, test plugins |
| `lumit-aplug`, `-aplug-broker`, `-aplug-testplug` | The same for CLAP and VST3 audio plugins |
| `lumit-bench` | The performance harness CI runs |
| `lumit-bridge` | The seam Flutter calls. Package name `lumit_bridge` |

Built-in effects live in `crates/lumit-core/src/fx/effects/`.

Dependencies point down only: bridge, then engine crates, then `lumit-core`. No engine
crate knows a UI exists. `lumit-core` has no GPU, codec or audio dependency.

| Outside `crates/` | What it is |
|---|---|
| `flutter_ui/` | The app |
| `web/`, `web-docs/` | lumitlab.com and docs.lumitlab.com (Astro). Nothing depends on them |
| `scripts/` | `build`, `check`, `codegen`, `translations`, `manual-pages`, `shots`, `win-dev-env`. Each answers `-?` |
| `packaging/` | Windows installer, macOS dmg, Linux desktop files, Flatpak |
| `tools/` | The After Effects export script and its audit kit |
| `.github/workflows/` | `ci.yml`, and `release.yml` on a `v*` tag |
