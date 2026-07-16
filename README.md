# Luminal

A native motion-graphics and compositing editor — After Effects' depth, Vegas' retiming
soul, one application. Built first for gaming-edit and montage editors; growing into a full
After Effects replacement. Rust · wgpu · egui · GPLv3.

**Status: design phase.** The complete system is specified in [docs/](docs/) before the
first line of application code; the specs are canonical and implementation follows them.

## Why

The montage scene edits in After Effects plus an expensive third-party plugin stack, and
lives with preview lag, crashes, and a retiming workflow many of them fight. Luminal's
promises: playback at speed, degrade-never-crash, retiming as a first-class citizen with a
beat-sync covenant, and the genre's staple effects in the box. The full pitch:
[docs/00-VISION.md](docs/00-VISION.md).

## The documentation set

| Doc | What it specifies |
|---|---|
| [00-VISION](docs/00-VISION.md) | Why Luminal exists, pillars, non-goals, the v1 milestone |
| [01-GLOSSARY](docs/01-GLOSSARY.md) | Canonical terminology — binding on all docs, UI, and code |
| [02-DECISIONS](docs/02-DECISIONS.md) | Numbered decision log with rationale |
| [03-DATA-MODEL](docs/03-DATA-MODEL.md) | Project/comp/layer/clip/property/keyframe object model |
| [04-RETIMING](docs/04-RETIMING.md) | The Retime system: segments, two graph lenses, the covenant |
| [05-ARCHITECTURE](docs/05-ARCHITECTURE.md) | Crates, threads, document snapshots, evaluation graph, GPU |
| [06-RENDER-PIPELINE](docs/06-RENDER-PIPELINE.md) | Render order, colour, caching, preview, export |
| [07-UI-SPEC](docs/07-UI-SPEC.md) | Panels, workspaces, Viewer, Timeline, graph editor, keymap |
| [08-EFFECTS](docs/08-EFFECTS.md) | Built-in effect suite (the montage staples in-box) |
| [09-AUDIO](docs/09-AUDIO.md) | v1 sync toolkit; the future Composer |
| [10-FILE-FORMAT](docs/10-FILE-FORMAT.md) | The .lum container, sidecar caches, autosave |
| [11-AE-IMPORT](docs/11-AE-IMPORT.md) | After Effects project import and the fidelity matrix |
| [12-PLUGINS](docs/12-PLUGINS.md) | OFX hosting, the KFX native API, expressions |
| [13-PERFORMANCE-RULES](docs/13-PERFORMANCE-RULES.md) | Budgets, resource governor, degradation ladder |
| [14-ENGINEERING-RULES](docs/14-ENGINEERING-RULES.md) | Binding rules for all code |
| [15-DESIGN](docs/15-DESIGN.md) | Dark-first Aizome design language |
| [16-ROADMAP](docs/16-ROADMAP.md) | Phases and their gates |

Three companion pieces:
- [docs/GUIDE.md](docs/GUIDE.md) — the plain-English guide to the codebase: what each crate
  does, Rust and threading explained in editing terms, and the safe-change recipe. Start
  here if you aren't a Rust developer.
- [docs/impl/](docs/impl/) — implementation notes for the genuinely hard, low-level parts
  (rational time, cubic solving, wgpu patterns, hardware decode interop, the scheduler,
  optical flow, OFX hosting, beat detection, expression embedding): exact algorithms,
  reference code, traps, and test plans.
- [docs/research/](docs/research/) — the research notes that informed the specs.

## Building

Luminal builds on Windows, macOS and Linux with the stable Rust toolchain. The one outside
dependency is **FFmpeg 7.x** (video/audio decode), plus **LLVM 18** on Windows for the
binding generator.

- **macOS**: `brew install ffmpeg@7`, then `cargo test --workspace`. The repo's
  `.cargo/config.toml` points the build at the keg.
- **Windows**: unzip a [BtbN FFmpeg 7.1 shared/GPL build](https://github.com/BtbN/FFmpeg-Builds/releases)
  under `%USERPROFILE%\ffmpeg\`, `winget install LLVM.LLVM --version 18.1.8`, then
  `. .\scripts\win-dev-env.ps1 -Persist` to wire it up. `cargo run -p luminal-app` launches.
- **Linux** (K-082): install the FFmpeg 7 development packages plus `pkg-config` and `clang`,
  then `cargo run -p luminal-app`. Debian 13 / Ubuntu 24.10 or newer:
  `sudo apt install pkg-config clang libavcodec-dev libavformat-dev libavutil-dev libswscale-dev libswresample-dev libavfilter-dev libavdevice-dev`.
  Arch: `sudo pacman -S ffmpeg clang pkgconf`. Note that FFmpeg **7.x** is required —
  distributions still shipping FFmpeg 6 (Ubuntu 24.04 LTS among them) need a newer release
  or a self-built FFmpeg before Luminal will build.

Full step-by-step, in plain English: [docs/GUIDE.md](docs/GUIDE.md) §8.

## Licence

[GPLv3](LICENSE). Forks stay open; contributions welcome once implementation begins.
