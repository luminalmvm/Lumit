# Doing it yourself

Every routine this repository needs, written out so you can run it without asking
anyone. Each one says when you need it, what to type, what a good result looks
like, and the two ways it usually goes wrong.

[07-BUILD-SHIP.md](07-BUILD-SHIP.md) explains *why* the build is shaped the way
it is. This page is the *how*: commands, from the repository root, on your own
Windows machine.

## The scripts

Where a routine is more than a couple of commands there is a script for it in
`scripts/`. Each one prints every command before it runs it, in cyan, so you can
watch what it does and type the same thing yourself next time. None of them hides
anything: the raw commands are also written out below, and the scripts run
exactly those.

| Script | Does |
|---|---|
| `.\scripts\build.ps1` | Builds the engine, or just the bridge library |
| `.\scripts\check.ps1` | Formatting, clippy and the tests, as CI runs them |
| `.\scripts\codegen.ps1` | The whole bridge regeneration cycle, in the right order |
| `.\scripts\manual-pages.ps1` | The manual's effect pages, and optionally their pictures |
| `.\scripts\shots.ps1` | One screenshot sweep through the real application |

Every script accepts `-?` for its own help:

```powershell
.\scripts\codegen.ps1 -?
```

## First: the environment

**When you need it.** Once per terminal window, before anything that compiles
Rust. The media crate links FFmpeg, and three environment variables tell it
where FFmpeg and libclang are.

```powershell
. .\scripts\win-dev-env.ps1
```

The leading dot matters. It means "run this in *my* shell", so the variables it
sets survive; without it they are set inside a shell that then closes and takes
them with it.

**Success looks like** four lines naming `FFMPEG_LIBS_DIR`,
`FFMPEG_INCLUDE_DIR`, `LIBCLANG_PATH` and a `PATH` addition.

**When it goes wrong.**

- *"Could not find an FFmpeg 7.1 shared/GPL build."* You have no FFmpeg, or the
  wrong one. Download `ffmpeg-n7.1-latest-win64-gpl-shared-7.1.zip` from
  [BtbN's builds](https://github.com/BtbN/FFmpeg-Builds/releases) and extract it
  under `%USERPROFILE%\ffmpeg\`. The word `shared` is load-bearing: a static
  build has no `bin\` full of DLLs and nothing will link.
- *"Could not find libclang.dll."* `winget install LLVM.LLVM --version 18.1.8`.
  Version 18 specifically — the tool that reads FFmpeg's C headers produces
  broken output against much newer versions of LLVM.

Pass `-Persist` once and every future terminal inherits the variables, at which
point you can forget this section exists. The scripts in `scripts/` dot-source
this file themselves, so they are safe to run in a fresh terminal either way.

## Building the engine

**When you need it.** Before `flutter test`, and any time you want to know
whether the Rust side still compiles. You do **not** need it to run the app:
`flutter run` compiles the bridge crate as part of its own build.

```powershell
.\scripts\build.ps1                 # the whole workspace, debug
.\scripts\build.ps1 -BridgeOnly     # just the library the Flutter tests load
.\scripts\build.ps1 -Release        # optimised
```

Raw:

```powershell
cargo build --workspace
cargo build -p lumit_bridge
cargo build --workspace --release
```

**Success looks like** `Finished ... target(s) in 14.28s` and a file at
`target\debug\lumit_bridge.dll`.

**A debug build here is not an unoptimised build.** The root `Cargo.toml` gives
`lumit-core`, `lumit-eval`, `lumit-gpu` and `lumit-render` full optimisation in
both profiles, because that is where the hot loops live and a genuinely
unoptimised lens flare bake measured about sixteen times slower. `--release`
still buys you the rest of the workspace and removes the debug assertions, and
it is what performance claims are measured against.

**When it goes wrong.**

- *`error: package ID specification 'lumit-bridge' did not match any packages`.*
  The folder is `crates/lumit-bridge` with a hyphen, but the package inside it is
  named `lumit_bridge` with an underscore. Cargo wants the package name.
- *Link errors mentioning `avcodec` or `avformat`.* You skipped the environment
  step above, or opened a new terminal since. Dot-source `win-dev-env.ps1` again.

## Running the app

**When you need it.** To see a change.

```powershell
cd flutter_ui
flutter run -d windows
```

The first run is slow — it compiles the Rust bridge and the C++ runner from
nothing, several minutes — and every run afterwards is quick. While it is
running, `r` hot-reloads the Dart side, `R` restarts it, and `q` quits. Changes
to Rust need a full restart of the command, because the compiled library is
loaded once.

**Success looks like** the editor window opening with an empty project.

**`LUMIT_SHOTS`** has nothing to do with normal running. It is the guard on the
screenshot sweeps described further down: those sweeps are the application
started on a different entry point, and without `LUMIT_SHOTS=1` they print one
line and quit. That is deliberate, so nothing automatic can find itself driving
the editor and overwriting the manual's pictures.

**When it goes wrong.**

- *`No supported devices connected`.* Windows desktop support is off. Run
  `flutter config --enable-windows-desktop` once, then `flutter doctor` to see
  what else it wants. Visual Studio with the C++ desktop workload is required —
  the runner is a C++ program.
- *The window opens, then closes immediately, or the console mentions a missing
  DLL.* FFmpeg is not on `PATH` for that terminal. Dot-source `win-dev-env.ps1`
  and run again.

## When you change the bridge API

**When you need it.** Whenever you edit anything under
`crates/lumit-bridge/src/api/`. That folder declares, in Rust, everything the
Flutter side is allowed to call. A generator reads it and writes both halves of
the seam: the Rust glue, and the Dart classes the app calls.

```powershell
.\scripts\codegen.ps1
```

Raw, and the order matters:

```powershell
cd flutter_ui
flutter pub get
flutter_rust_bridge_codegen generate
cd ..
cargo build -p lumit_bridge
git status
```

The generator must run from `flutter_ui/`, because that is where
`flutter_rust_bridge.yaml` lives and every path in it is relative to that file.

**Success looks like** `Done!` from the generator, then a `git status` showing
changes under `flutter_ui/lib/src/rust/` and
`crates/lumit-bridge/src/frb_generated.rs` — or nothing at all, if your edit did
not change the surface.

**Never edit a generated file.** Anything under `flutter_ui/lib/src/rust/` is
written by the generator, CI regenerates it and compares, and a hand edit is
silently undone by the next run. Change `api/**` and regenerate.

**The rebuild in step four is the one people forget.** The Dart tests under
`flutter_ui/test/frb/` do not build the engine — they *load*
`target/debug/lumit_bridge.dll` and drive the real thing. The two sides compare a
content hash at startup, so a library that no longer matches the generated Dart
refuses to start. That failure happens inside the setup of every test, before any
widget is built, so what you actually see is a screen of
`Expected: exactly one matching candidate / Actual: _WidgetTypeFinder:<zero
widgets with type ...>` — "found 0 widgets", over and over. It reads like a
broken interface and it is not: it means the library is stale. Rebuild it.

**When it goes wrong.**

- *`flutter_rust_bridge_codegen` is not recognised.* Install it at the pinned
  version — the one in `crates/lumit-bridge/Cargo.toml`, currently 2.12.0:
  `cargo install flutter_rust_bridge_codegen --version 2.12.0 --locked`. A
  different version writes a different tree and CI's freshness check fails.
- *CI's `codegen-fresh` job fails but regenerating locally changes nothing.* Part
  of the generated output quotes names from the Rust compiler's own expansion, so
  generating on a different Rust release can move a comment line. Update your
  toolchain to current stable, regenerate, commit that.

## Running tests

**When you need it.** Before every commit, on whatever you touched.

The engine, one crate at a time — this is the normal case while working:

```powershell
cargo test -p lumit-core
cargo test -p lumit-render
cargo test -p lumit-gpu -- --test-threads=1
```

The GPU crate is always single-threaded. Its tests share one graphics device and
tread on each other when run in parallel.

Everything, the way CI does it:

```powershell
.\scripts\check.ps1              # formatting, clippy, then all the tests
.\scripts\check.ps1 -Crate lumit-core
```

Raw:

```powershell
cargo test --workspace --exclude lumit-gpu
cargo test -p lumit-gpu -- --test-threads=1
```

To run a single test by name, give cargo part of the name:

```powershell
cargo test -p lumit-core the_fx_reference_fixture_matches_the_catalogue
```

The Flutter side, **one file at a time**:

```powershell
cd flutter_ui
flutter test test/theme_test.dart
flutter test test/frb/timeline_panel_frb_test.dart
```

**Never run the whole Flutter suite on this machine.** `flutter test` with no
file argument starts a test process per file, in parallel, and each of the `frb`
ones loads the engine and a graphics device. It has frozen this machine hard
enough to need the power button. CI runs the full suite with `--concurrency=1` on
its own hardware; that is what it is for. Locally, name the files you changed.

**Success looks like** `All tests passed!` for Flutter, or
`test result: ok. 551 passed` for cargo.

**When it goes wrong.**

- *Every frb test reports "found 0 widgets".* The engine library is stale or
  missing. `cargo build -p lumit_bridge`, then run the test again. See the
  section above for why this is what a stale library looks like.
- *`The engine library is not built.`* The same problem, said plainly, because
  the file is absent rather than out of date. Same fix.

## Formatting and clippy

**When you need it.** Before every commit. Both are merge gates and neither is
advisory.

```powershell
cargo fmt --all --check          # complains
cargo fmt --all                  # fixes
cargo clippy --workspace --all-targets -- -D warnings
cd flutter_ui; flutter analyze
```

Or `.\scripts\check.ps1 -SkipTests` for the first three, and `-Fix` to let the
formatter rewrite rather than complain.

**Success looks like** silence from `fmt --check`, and `Finished` with no
warnings from clippy. `-D warnings` means a warning ends the run, which is the
policy: this workspace also denies `unwrap`, `expect`, `panic!`, `todo!` and
`unsafe`, so an unfinished branch does not compile.

**When it goes wrong.**

- *Clippy complains about `unwrap()` in test code.* Tests are allowed it, but
  they must say so: the modules that use it carry
  `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]` at the top
  of the `#[cfg(test)] mod tests` block. Copy that line from a neighbouring file.
- *Clippy passes locally but CI's `no-panics-in-frb-api` job fails.* That job is
  a plain text search over `crates/lumit-bridge/src/api/**`, because the bridge
  macro hides those calls from clippy. Handle the error instead.

## The effect catalogue

**When you need it.** After adding an effect, or changing any parameter's name,
range, default or unit. The catalogue is a JSON file the engine writes about
itself, and the manual's tables are generated from it, so it must be current or
the manual describes controls the engine no longer has.

```powershell
cargo test -p lumit-core regenerate_fx_reference -- --ignored
```

**Success looks like** `test result: ok. 1 passed`, and `git status` showing
`crates/lumit-core/fx-reference.json` changed — or clean, if nothing moved.

The odd `-- --ignored` is Rust's way of saying "this one only runs when asked for
by name". The test writes a file, so it must not fire on every run.

**When it goes wrong.**

- *`the_fx_reference_fixture_matches_the_catalogue` fails.* That is the guard,
  not a bug: it fails precisely because the file is now stale. Its own message
  tells you the command above. Run it and commit the result.
- *You changed a slider and the JSON did not move.* You changed the widget rather
  than the declaration. The numbers live on the effect's own declaration in Rust,
  which is the only place they are written down.

## The manual's effect pages

**When you need it.** After regenerating the catalogue above. The pages are built
from it.

```powershell
.\scripts\manual-pages.ps1          # regenerate the catalogue, then the pages
.\scripts\manual-pages.ps1 -Check   # change nothing; fail if the pages are stale
```

Raw:

```powershell
cargo test -p lumit-core regenerate_fx_reference -- --ignored
cd web-docs
npm install          # first time only
npm run docs:effects
```

**Success looks like** a list of the pages it wrote, or `no changes - pages
already match the catalogue` when nothing moved. `-Check` says `effect pages are
up to date` and exits zero.

The generator does not own the pages. Each page has a marked-off block between
two invisible comments, and only what is between those marks is rewritten. The
prose — what the effect is for, what each control does, the warnings — is
hand-written above and below and survives every regeneration. An effect with no
page yet gets a whole one scaffolded, waiting for someone to write the prose.

**When it goes wrong.**

- *An effect is reported as orphaned.* A page exists for an effect the engine no
  longer has, usually after a rename. Delete the old page, or move its prose into
  the new one.
- *`npm` is not recognised, or `Cannot find module 'sharp'`.* Install
  [Node](https://nodejs.org) and run `npm install` inside `web-docs/` once.

## The manual's effect pictures

**When you need it.** Rarely. When an effect's *look* changes, or a new effect
needs its before-and-after figure. It takes a few minutes and needs ffmpeg.

```powershell
.\scripts\manual-pages.ps1 -Pictures
.\scripts\manual-pages.ps1 -Pictures -Only accumulation_mb   # redo just one
```

Raw, from `web-docs/`:

```powershell
npm run docs:effect-shots
```

Nobody draws these. The engine renders them, through the same walk the Viewer
uses, so the figure on a page is a frame the application would actually make.
Three steps happen inside that one command: ffmpeg turns the source photo into a
short panning shot, so effects that read motion have something to read; a cargo
test renders one frame per effect into raw pixels; and those are encoded to WebP
under `web-docs/src/assets/effects/`.

**ffmpeg must be on `PATH`** — `win-dev-env.ps1` puts it there, and the script
dot-sources that for you.

**About the source footage.** There are two committed stills,
`web-docs/src/assets/effect-plate.jpg` and `effect-depth.png`: one frame of a
game recording, and the depth pass from the same render. They are the fallback,
and everything except a handful of motion effects looks identical rendered from
them. The better inputs are real footage — `effect-plate.mp4`, `effect-depth.mp4`
and a graded `effect-lut.cube` beside them — which are **gitignored**, because a
repository is not the place for video and the LUT is not ours to redistribute.
If you have them, drop them in that folder and the script prefers them
automatically, printing `using the recorded clips`. A fresh clone prints `no
recorded clip; panning the committed stills instead`, and that is fine.

**When it goes wrong.**

- *`ffmpeg: The system cannot find the file specified.`* ffmpeg is not on `PATH`.
  Dot-source `win-dev-env.ps1`.
- *`no source at ...effect-plate.mp4 and none at ...effect-plate.jpg`.* You are
  running from somewhere the committed stills are missing. Check you are on a
  full checkout rather than a sparse one.

## The app's screenshots

**When you need it.** When a panel's design changes and the manual's picture of
it goes out of date.

```powershell
.\scripts\shots.ps1 -Sweep 2
.\scripts\shots.ps1 -Sweep retakes -Shape round -Out C:\tmp\shots-review
```

Raw:

```powershell
cargo build -p lumit_bridge
cd flutter_ui
$env:LUMIT_SHOTS = '1'
flutter run -d windows -t tool/shots/shots_2.dart
```

A sweep is not a test. It is the application itself, started on a different entry
point: it boots exactly the way `lib/main.dart` does, stages a plausible project
through the real engine, photographs its own window from outside, and quits. What
the manual shows is therefore the program, not a harness impersonating it. (The
obvious approach, an integration test, was tried and abandoned: on this version
of Flutter the test-driven window photographs as a plain white rectangle.)

There are seven numbered sweeps plus `retakes` and `round_v2`; what each one
covers is written at the top of its own file in `flutter_ui/tool/shots/`.
Finished pictures land in `web-docs/src/assets/shots/`, and `-Out` sends them
somewhere else so a pass can be reviewed before it overwrites anything. The
script creates that folder; a sweep does not, and writes its first picture the
moment it has one, so a raw run pointed at a folder that does not exist yet dies
mid-pass with `PathNotFoundException`.

`LUMIT_SHOTS=1` is the guard, and the other three variables are the knobs:
`LUMIT_SHOTS_SHAPE` picks the theme shape to photograph in, `LUMIT_SHOTS_OUT`
redirects the output, and `LUMIT_SHOTS_NOCROP` keeps the whole window when a crop
is landing in the wrong place. A sweep writes into a throwaway settings file, so
it can never show your own panel arrangement or overwrite your settings.

**Success looks like** the editor opening, moving through several states on its
own, and closing, with new PNG files in the output folder.

**When it goes wrong.**

- *`SKIPPED: a screenshot sweep runs only with LUMIT_SHOTS=1 set.`* Exactly what
  it says. The script sets it for you; if you are running the raw command, set it
  first.
- *The app opens and the layers are empty, or it fails looking for a file.* The
  sweeps need media fixtures in `C:/tmp/lumit-shots` — `Gameplay.mp4`,
  `Title card.mp4`, `Music.wav` and `Logo.png`. They are not committed, for the
  same reason the plate footage is not. Make them once with ffmpeg; any short video
  with those names will do.

## The two websites

**When you need it.** To change lumitlab.com (`web/`) or the manual
(`web-docs/`). Both are small Astro sites, they live outside the Cargo workspace,
and nothing in the application depends on them.

```powershell
cd web            # or web-docs
npm install       # first time only
npm run dev       # live-reloading preview at the address it prints
npm run build     # writes ./dist, and fails on broken links or bad frontmatter
npm run preview   # serves what build wrote
```

**Success looks like** `6 page(s) built` for `web/`, or `138 page(s) built` for
`web-docs/`, and `Complete!`.

Deployment is not something you run. Cloudflare watches the repository and builds
each site on push; other branches get preview URLs. There is no `wrangler deploy`
in the normal flow.

**When it goes wrong.**

- *The build fails on a release note.* The frontmatter schema is strict, and the
  filename is an interface: `web/src/content/releases/0.2.0.md` serves
  `/releases/0.2.0` and is the exact path the Discord announcement reads. No
  leading `v`, no renaming.
- *A new docs page builds but belongs to no sidebar topic.* Most sections
  generate themselves from their folder, but the five groups under "Using Lumit"
  list their pages by hand in `web-docs/astro.config.mjs`. Add a line there.

## Translations

**When you need it.** Whenever you add, rename or reword a user-facing string.

Every string in the application lives in `flutter_ui/lib/l10n/app_en.arb`, in
British English, with an `@key` entry beside it describing where it appears — a
translator sees the phrase, not the screen, so that description is the only
context they get. Widgets read `l10n.someKey` and never contain the text itself.

There is no command to run for the English side: `flutter pub get`, and every
build, regenerates the Dart from the file.

**What an arb change means.**

1. The other `app_*.arb` files are now short of a key. That is expected and
   handled — a missing translation falls back to English — but it is not
   automatic. **Say so in the commit message and in the pull request**, listing
   the new keys, or the upload gets forgotten and the string stays English in
   every other language for a whole release.
2. The source has to go up to Crowdin: `crowdin push sources`, which reads
   `crowdin.yml` and needs `CROWDIN_PROJECT_ID` and `CROWDIN_PERSONAL_TOKEN` in
   the environment. Translations come back down with `crowdin pull translations`,
   or arrive by themselves on the `translation/main` branch.
3. **Never hand-edit `app_de.arb`, `app_uk.arb`, `app_zh.arb`, `app_zh_Hant.arb`
   or `app_kk.arb`.** Those are Crowdin's. A fix typed here is overwritten by the
   next sync. Fix it on Crowdin instead.

**A string the engine sends needs two entries, not one.** Effect labels, category
names and keymap action descriptions are English text that Rust hands over, so
they need the `app_en.arb` key *and* a matching entry in
`flutter_ui/lib/l10n/engine_labels.dart`. `engine_labels_test.dart` walks the
engine's own tables and fails on any gap. It is easy to add a keymap action
without thinking of it as "a string".

**When it goes wrong.**

- *`flutter pub get` fails complaining that a locale and a filename disagree.* A
  Crowdin sync wrote `zh-CN` into `app_zh.arb`. A workflow mends this on the sync
  branch automatically; if you hit it another way, the `@@locale` value must
  match the filename.
- *CI's Flutter job fails on `engine_labels_test`.* You added something the
  engine names in English without adding it to `engine_labels.dart`.

## macOS and Linux, honestly

Neither is built on this machine, and nothing below has been run here. What
follows is read off `.github/workflows/ci.yml`, which is the only place these
platforms are exercised.

**Linux is genuinely built and tested.** Two jobs cover it. `linux` runs the full
engine test suite, plus the GPU tests on a software Vulkan driver with
`LUMIT_REQUIRE_GPU=1` — meaning a skipped GPU test counts as a failure there —
plus a build of the bridge with its media feature switched off.
`flutter-linux` runs `flutter analyze`, builds the bridge, runs the **whole**
Flutter suite with `--concurrency=1`, and finishes with
`flutter build linux --release`. So the Linux application is compiled and its
tests pass on every commit. The shipped artefact is a single-file Flatpak.

**macOS is compiled, not exercised.** The `check` job runs the engine's tests
there, so the Rust side is genuinely tested on macOS. The application is another
matter: `flutter-macos` runs `flutter build macos --release` and stops. That
build is a real gate — it resolves the CocoaPods spec, compiles the Swift runner
and links the bridge — but it only proves the thing compiles. `flutter analyze`
and the Flutter tests are deliberately not repeated there, because Linux already
runs both and neither is platform-specific.

**What is therefore untested on macOS**, in plain terms:

- **Nobody has launched the .app.** The macOS Viewer takes a different path to
  the screen from the Windows one, and that path has never been seen working.
- **The bundle is not relocatable.** It links FFmpeg by absolute path from the
  build machine's Homebrew installation, so it runs on a machine set up like the
  CI runner and not necessarily on anyone else's.
- **It is single-architecture**, pinned to whatever the runner is. A universal
  bundle needs work that has not been done.

If you want to build for either platform yourself, the honest answer is that the
recipe is the workflow file. Read the job you care about in
`.github/workflows/ci.yml` — it lists every dependency it installs and every
command it runs, in order — and follow it. Do not trust a set of steps invented
for a platform nobody has tried them on.
