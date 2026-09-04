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
| `.\scripts\translations.ps1` | Reads in a translation file, and says where each language stands |

Every script accepts `-?` for its own help:

```powershell
.\scripts\codegen.ps1 -?
```

Three more in the same folder are not PowerShell. `scripts\gen-icons.py` and
`scripts\check-icon.py` are Python, because they are picture work — the first
re-renders every raster icon from the SVG sources, the second guards the macOS
icon document; both have their own section further down.
`scripts\discord-release.mjs` is Node, and you do not run it: the `announce` job
in `.github/workflows/release.yml` runs it on a release tag, reading the very
release note the website serves so the two cannot disagree.

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

**The workspace has two products, not one.** `lumit_bridge.dll` is the engine the
app loads; `target\debug\lumit-ofx-broker.exe` is a small separate program that
opens one OFX plugin bundle in a process of its own, so a third-party plugin that
crashes takes nothing with it. `cargo build --workspace` makes both.
`-BridgeOnly` stops at the library, which is all `flutter test` needs and not
enough to open an OFX plugin.

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

The first run is slow — it compiles the Rust bridge, the OFX broker and the C++
runner from nothing, several minutes — and every run afterwards is quick. While
it is running, `r` hot-reloads the Dart side, `R` restarts it, and `q` quits.
Changes to Rust need a full restart of the command, because the compiled library
is loaded once.

`flutter run` builds more of the workspace than it looks like it does.
`flutter_ui/windows/CMakeLists.txt` builds `lumit_bridge` **and**
`lumit-ofx-broker`, and installs the broker's executable beside the app's own,
because that is where the host looks for it. The Linux CMake file does the same.
So a change to the broker needs a fresh `flutter run`, not a hot reload.

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
cargo test -p lumit-gpu
```

The GPU tests need no special treatment. They share one graphics device and
one set of compiled shaders per test process, taking turns on it, so the
`lumit-gpu` crate runs in under a minute and `lumit-render` in a few
([GUIDE.md](../GUIDE.md) covers the arrangement). It used to be hours,
because every test opened the card and compiled every shader for itself.

Everything, the way CI does it:

```powershell
.\scripts\check.ps1              # formatting, clippy, then all the tests
.\scripts\check.ps1 -Crate lumit-core
```

Raw:

```powershell
cargo test --workspace
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
enough to need the power button. CI runs the full suite on its own hardware; that
is what it is for. Locally, name the files you changed.

**Success looks like** `All tests passed!` for Flutter, or
`test result: ok. ... passed; 0 failed` for cargo — one such line per crate, and
`.\scripts\check.ps1` prints `All green.` after the last of them.

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

Or `.\scripts\check.ps1 -SkipTests` for the Rust pair, and `-Fix` to let the
formatter rewrite rather than complain. `flutter analyze` is not in that script:
it is the Flutter side's own gate and needs no Rust environment, so it stays a
one-liner you run from `flutter_ui/`.

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

## Plugins other people wrote

**When you need it.** After touching anything under `crates/lumit-ofx/` or
`crates/lumit-ofx-broker/`. Lumit hosts OFX plugins — the format Resolve, Nuke
and Natron all speak — and the risk in that is other people's C code running
inside ours, which is why it runs in the broker's process instead.

The ordinary suites cover it and need nothing special:

```powershell
cargo test -p lumit-ofx
cargo test -p lumit-ofx-broker
```

Those use `lumit-ofx-testplug`, a plugin written here on purpose, so they need
no download. Two heavier checks exist and both live in CI rather than in the
routine:

```powershell
cargo run --release -p lumit-bench --bin ofx-bench
cargo test -p lumit-ofx --test conformance -- --nocapture
```

The first fetches and builds two real plugin suites, openfx-misc and ntsc-rs,
into `target/ofx-bench/`; the second describes, instances and renders every
plugin in it and writes a pass/fail table to `target/ofx-conformance.md`. It is
about eighty plugins and a C++ toolchain — an afternoon the first time, cached
after that — so run it when you have changed how the host answers a plugin, not
before every commit.

**`handle_fuzz` is a nightly job**, not a local one. CI runs it under
AddressSanitizer, which needs an unstable compiler flag; `cargo test -p lumit-ofx
--test handle_fuzz` on stable runs the same target unsanitised, which is a fast
check that a forged handle is refused, and not the check that refusing it also
reads nothing it should not.

## The application's icons

**When you need it.** Only after editing an SVG in `assets/brand/`. The drawings
are the only artwork anyone touches; the operating systems want pixels.

```powershell
pip install resvg-py pillow      # first time only
python scripts\gen-icons.py
python scripts\check-icon.py     # the macOS icon document, guarded
```

`gen-icons.py` renders each SVG at every size an `.ico` or `.icns` needs — each
size straight from the drawing rather than scaled down from a big render, which
is what keeps the small ones crisp — and writes the results, all of which are
committed. Its own header lists exactly which files it produces.

The macOS **application** icon is not made there. It is the layered Icon
Composer document `assets/brand/lumit-icon.icon`, which Xcode compiles during
`flutter build macos`, and `check-icon.py` exists because Icon Composer can
write two settings into it that make Apple's `actool` crash part-way through
with a message that says nothing about the cause. CI runs that check;
run it yourself after opening the document in Icon Composer.

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

There are seven numbered sweeps and five named ones — `graph`, `modes`,
`retakes`, `round_v2` and `welcome`. What each one covers is written at the top
of its own file in `flutter_ui/tool/shots/`, and a name the folder does not have
is refused by the script with the full list.
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
- *`CROP CLAMPED <shot>: … Npx of it is not on screen.`* The sweep asked for a
  region that runs off the edge of the window, and the shot was cut short at the
  edge rather than moved. Usually the floor of the crop came from a row that the
  staging has since pushed below the fold. The picture is of the right place with
  a piece missing, so it is worth fixing but it is not misleading.

  **This one used to be silent, and that is the trap worth knowing about.**
  ffmpeg does not refuse a crop that runs off the picture — it slides the whole
  rectangle back inside and crops *there*, exits 0, and says nothing. A shot
  could therefore be of somewhere else entirely while every log line looked
  right; `shape-layer.png` was a picture of the Viewer for exactly this reason.
  `captureUi` now intersects the crop with the picture and prints the line above,
  so the failure announces itself.
- *`CROP OFF-PICTURE <shot>`.* The same thing, all the way: nothing the crop
  asked for is on screen, so the whole window was photographed instead. Always a
  staging mistake — go and look at what the shot is aiming at.
- *A control is sliced down the middle at the edge of a panel shot.* Nearly
  always `boxOfType` rather than `spanOfType`. A widget that is not itself a
  render object has no box of its own, so `Element.renderObject` hands back the
  first descendant box it finds — often an inner one, short of both the panel's
  real width and its foot. `spanOfType` unions the whole subtree instead, and is
  what every whole-panel crop should use.

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

**Success looks like** `7 page(s) built` for `web/`, or `145 page(s) built` for
`web-docs/`, and `Complete!`. Both numbers grow with the content; what matters is
that the build finishes rather than what it counts.

`web-docs/` has one more command, `npm test`, which runs the generators' own
unit tests (`node --test scripts/*.test.mjs`). Run it if you change anything
under `web-docs/scripts/`.

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
2. The new English reaches translators through the translation page on
   lumitlab.com, which is built from `app_en.arb` itself — so a key that lands in
   the repository is offered the next time the site is deployed. What a translator
   sends back is read in by `scripts/translations.ps1`, which is the only thing
   that writes a translation file.
3. **Never hand-edit `app_de.arb`, `app_es.arb`, `app_kk.arb`, `app_pl.arb`, `app_uk.arb`,
   `app_zh.arb` or `app_zh_Hant.arb`.** Those belong to the ingest tool. A fix typed here is
   overwritten by the next run; make it on the translation page instead.

**When a translation arrives.** Somebody opens an issue with a `.json` from
lumitlab.com/translate. Download it and run the two commands:

```powershell
.\scripts\translations.ps1 ingest .\lumit-de.json
.\scripts\translations.ps1 status
```

The first reads the file and refuses the whole of it if anything is wrong — an
unknown language, a key `app_en.arb` does not have, a sentence that dropped a
`{placeholder}` — so a bad line never lands half-applied. What it accepts goes
into `app_de.arb`, and the English each entry was translated from goes into
`flutter_ui/lib/l10n/translation-state.json` beside it. The second prints where
every language stands: translated, stale, missing. Commit both files together and
say in the message which language gained how many strings.

Two more, run rarely. `seed` records today's English for translations already in
the files and is safe to run again — it only fills in what the sidecar is missing.
`prune` deletes translations of keys English no longer has, and expires the stale
ones (a translation whose English moved on falls back to English rather
than answering the old question), so run it after a sweep that rewords strings.
`.\scripts\translations.ps1 -SelfTest` runs the whole round trip against a
throwaway folder in `%TEMP%` and touches nothing here.

**A string the engine sends needs two entries, not one.** Effect labels, category
names and keymap action descriptions are English text that Rust hands over, so
they need the `app_en.arb` key *and* a matching entry in
`flutter_ui/lib/l10n/engine_labels.dart`. `engine_labels_test.dart` walks the
engine's own tables and fails on any gap. It is easy to add a keymap action
without thinking of it as "a string".

**When it goes wrong.**

- *`flutter pub get` fails complaining that a locale and a filename disagree.*
  Something wrote `zh-CN` into `app_zh.arb`. The `@@locale` value inside a file
  must be exactly what its filename says; `arb_test.dart` checks it on every run,
  so the failure names the file.
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
Flutter suite, and finishes with `flutter build linux --release`. So the Linux
application is compiled and its tests pass on every commit. The shipped artefact
is a single-file Flatpak.

That suite runs at Flutter's ordinary parallelism now. `--concurrency=1` used to
be there, because every test process leaked an engine worker and a GPU device
and the runner ran out; closing the project reference ended that, and the files
run together. It also runs with `LUMIT_NO_ZERO_COPY_VIEWER=1`, because the
runner has no GPU and the software Vulkan driver refuses the shared allocation
the Viewer's zero-copy path needs — so the handful of tests that wait for a
picture skip there and nowhere else.

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
