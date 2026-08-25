[English](./README.md) | [中文](./README.zh-CN.md) |
<div align="center">

<a href="https://lumitlab.com">
<img src="assets/brand/lumit-mark.svg" alt="lumitlab.com" width="96">
</a>

# Lumit

**A native motion-graphics and compositing editor.**
After Effects' depth, Vegas' retiming, one application. Free and open source.

[![CI](https://github.com/luminalmvm/Lumit/actions/workflows/ci.yml/badge.svg)](https://github.com/luminalmvm/Lumit/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/luminalmvm/Lumit?sort=semver&label=release)](https://github.com/luminalmvm/Lumit/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/luminalmvm/Lumit/total?label=downloads)](https://github.com/luminalmvm/Lumit/releases)
[![Crowdin](https://badges.crowdin.net/lumit/localized.svg)](https://crowdin.com/project/lumit)
[![Licence: GPL v3](https://img.shields.io/badge/licence-GPLv3-blue)](LICENSE)

[Website](https://lumitlab.com) ·
[Download](https://lumitlab.com/download) ·
[Documentation](https://docs.lumitlab.com) ·
[Releases](https://lumitlab.com/releases) ·
[Roadmap](docs/16-ROADMAP.md)

</div>

<!-- A screenshot of the editor goes here. -->

## What is Lumit

Lumit aims to bring the best of After Effects and Vegas to provide a way to
composite, cut, and retime all in a single editor. We hope to bring in an
audio editing view, as well as node editing in the future as well to let you
work your way.

Lumit was built as an alternative to after effects. The goal of which is a 
responsive application, that no matter the number of keyframes or layers in
your project, doesn't slow down to a crawl and become unresponsive.

We want to keep this open-source to allow anyone in the scene to contribute and
help support other's who want to create. Please bear in mind Lumit is still 
very early-access, if you discover bugs or issues, or even additional features
you want implemented, please raise an issue or work on it yourself and make a PR.

## Why it exists

This was made due to my issues frag editing in after effects and most of the time
spent working was waiting for previews. This originally was meant to be aimed at
frag and montage editors, but it has vastly expanded in scope to become a fully 
built out composite editor.

There were a couple of goals Lumit aims to provide for editors as well to make
it an editor you never need to leave:
- **Retiming with multiple options.** Whether you prefer After effect's time 
  remapping, or vegas' velocity, you can change the default graph view as you see
  fit, and sequence layers allow you to cut and splice clips together within a 
  single layer, whilst still allowing retiming per clip
- **Effect staples builtin** Glow, motion blur, camera shake, RGB
  split, smooth zoom, grades with a LUT loader, a physically-modelled lens
  flare. and many more. All built-in with no external plugins required. 
  OFX support, as well as our own custom plugin and scripting are planned.

## Installing

Installer's can be found at [lumitlab.com/download](https://lumitlab.com/download) or
the [latest GitHub release](https://github.com/luminalmvm/Lumit/releases/latest).
Lumit can check for updates and installs them automatically or when you want.

## Building from source

Rust stable (pinned by `rust-toolchain.toml`) plus two external dependencies:
**FFmpeg 7.x** for media, and **LLVM 18** for the binding generator — newer LLVM
silently generates broken bindings, so 18 is pinned on every platform.

<details>
<summary><b>Windows</b> (my primary development platform)</summary>

Unzip a [BtbN FFmpeg 7.1 shared/GPL build](https://github.com/BtbN/FFmpeg-Builds/releases)
under `%USERPROFILE%\ffmpeg\`, then:

```powershell
winget install LLVM.LLVM --version 18.1.8
. .\scripts\win-dev-env.ps1 -Persist
cargo test --workspace
```
</details>

<details>
<summary><b>macOS</b></summary>

```sh
brew install ffmpeg@7
# The formula is keg-only, so point the build at it (K-204):
export FFMPEG_PKG_CONFIG_PATH="$(brew --prefix ffmpeg@7)/lib/pkgconfig"
cargo test --workspace
```
</details>

<details>
<summary><b>Linux</b> (K-082)</summary>

FFmpeg needs no environment variable here — the development packages put their
`.pc` files where the build already looks.

```sh
# Debian 13 / Ubuntu 24.10 or newer
sudo apt install pkg-config clang libavcodec-dev libavformat-dev libavutil-dev \
  libswscale-dev libswresample-dev libavfilter-dev libavdevice-dev

# Arch / Artix — the unversioned clang is LLVM 19+ and produces broken bindings
sudo pacman -S ffmpeg pkgconf clang18 llvm18
```

If your default `clang` is newer than 18, point the build at 18 in the shell you
build from:

```sh
export LIBCLANG_PATH=/usr/lib/llvm18/lib          # Debian/Ubuntu: /usr/lib/llvm-18/lib
cargo test --workspace
```

FFmpeg **7.x** is required; distributions still on FFmpeg 6 (including Ubuntu
24.04 LTS) need a newer release or a self-built FFmpeg first.
</details>


The interface is in [flutter_ui/](flutter_ui/) and requires the Flutter SDK —
see [flutter_ui/README.md](flutter_ui/README.md). Step-by-step build notes in
plain English are in [docs/GUIDE.md](docs/GUIDE.md) §8.

## How the repository works

| | |
|---|---|
| [docs/README.md](docs/README.md) | The index — start here. Eighteen numbered specs, from the vision to the roadmap. |
| [docs/GUIDE.md](docs/GUIDE.md) | Used to be Plain English, no Rust assumed. What each crate does, and how to change things safely. However has become a monster of a file and some sections are worth ignoring completely. If you are looking for info I'd now recommend [docs.lumitlab.com](docs.lumitlab.com) |
| [docs/02-DECISIONS.md](docs/02-DECISIONS.md) | Every design decision with its reasoning, append-only. Search it, don't read it. |
| [docs/impl/](docs/impl/) | The implementation notes for more difficult area's. |
| [docs/TODO.md](docs/TODO.md) | What is next to work on now, next and later. |

The engine is a Cargo workspace under `crates/`; the interface is
`flutter_ui/`; they meet at `crates/lumit-bridge`
([17-BRIDGE-CONTRACT.md](docs/17-BRIDGE-CONTRACT.md)). `web/` and `web-docs/`
are the public site [lumitlab.com](lumitlab.com), and depend on nothing else here.

## Contributing

Issues and pull requests are welcome.

- [docs/01-GLOSSARY.md](docs/01-GLOSSARY.md) is binding on code, UI text and
  commit messages, please make sure to use the correct terms detailed here.
- Everything lands with tests, and CI runs must succeed.

Translators especially welcome: the interface is fully externalised but nothing
is translated yet. That work happens on
[Crowdin](https://crowdin.com/project/lumit), not in this repository — the only
language file edited here is the British-English source.

## Licence

[GPLv3](LICENSE). Forks stay open source.
