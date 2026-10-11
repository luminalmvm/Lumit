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
[![Licence: GPL v3](https://img.shields.io/badge/licence-GPLv3-blue)](LICENSE)

[Website](https://lumitlab.com) ·
[Download](https://lumitlab.com/download) ·
[Documentation](https://docs.lumitlab.com) ·
[Releases](https://lumitlab.com/releases)

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
**FFmpeg 8.x** for media, and **LLVM 18** for the binding generator. It has to be
FFmpeg 8, the build stops on 7 or 9. Newer LLVM silently generates broken
bindings, so 18 is pinned on Windows and Linux.

<details>
<summary><b>Windows</b> (my primary development platform)</summary>

Unzip a [BtbN FFmpeg 8.1 shared/GPL build](https://github.com/BtbN/FFmpeg-Builds/releases)
under `%USERPROFILE%\ffmpeg\`, then:

```powershell
winget install LLVM.LLVM --version 18.1.8
. .\scripts\win-dev-env.ps1 -Persist
cargo test --workspace
```
</details>

<details>
<summary><b>macOS</b></summary>

Homebrew's plain `ffmpeg` is 9.x, so install `ffmpeg@8`. It isn't linked into
your `PATH`, so point the build at it in the shell you build from:

```sh
brew install ffmpeg@8
export FFMPEG_PKG_CONFIG_PATH="$(brew --prefix ffmpeg@8)/lib/pkgconfig"
export PATH="$(brew --prefix ffmpeg@8)/bin:$PATH"   # the tests make their media with the ffmpeg CLI
cargo test --workspace
```

Xcode's own libclang works here, so there's no LLVM to install.
</details>

<details>
<summary><b>Linux</b></summary>

Ubuntu 26.04 ships FFmpeg 8, so there it's only packages:

```sh
sudo apt install pkg-config libclang-18-dev libasound2-dev libgl-dev libegl-dev \
  libxkbcommon-dev libxkbcommon-x11-dev libx11-dev libxcursor-dev libxi-dev \
  libxrandr-dev libxcb1-dev libwayland-dev \
  libavcodec-dev libavformat-dev libavutil-dev libswscale-dev libswresample-dev \
  libavfilter-dev libavdevice-dev
```

Everywhere else the distro's FFmpeg is the wrong one (7 on Debian 13 and older
Ubuntu, 9 on Arch), so leave its FFmpeg development packages out and unpack a
[BtbN FFmpeg 8.1 shared/GPL build](https://github.com/BtbN/FFmpeg-Builds/releases)
instead:

```sh
# Arch / Artix: sudo pacman -S pkgconf clang18 llvm18
mkdir -p ~/ffmpeg8
# The same dated build CI uses, checked against the same hash
f=ffmpeg-n8.1.2-50-g1a748fe2cd-linux64-gpl-shared-8.1.tar.xz
curl -fsSLO https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-08-31-13-27/$f
echo "35dc428bf78d3f8a4447a68707338ecf9560ebcc7459967974f9ff5b1f84be24  $f" | sha256sum -c -
tar -xJf $f -C ~/ffmpeg8 --strip-components=1
# The .pc files carry the prefix of the machine that built them
sed -i "s|^prefix=.*|prefix=$HOME/ffmpeg8|" ~/ffmpeg8/lib/pkgconfig/*.pc
export PKG_CONFIG_PATH="$HOME/ffmpeg8/lib/pkgconfig:$PKG_CONFIG_PATH"
export LD_LIBRARY_PATH="$HOME/ffmpeg8/lib:$LD_LIBRARY_PATH"
export PATH="$HOME/ffmpeg8/bin:$PATH"
```

Then point the build at LLVM 18 in the shell you build from, since the default
`clang` is newer on all of them:

```sh
export LIBCLANG_PATH=/usr/lib/llvm-18/lib          # Arch: /usr/lib/llvm18/lib
cargo test --workspace
```
</details>


The interface is in [flutter_ui/](flutter_ui/) and requires the Flutter SDK —
see [flutter_ui/README.md](flutter_ui/README.md). Step-by-step build notes are
in [docs/GUIDE.md](docs/GUIDE.md).

## How the repository works

| | |
|---|---|
| [docs/GUIDE.md](docs/GUIDE.md) | Start here. How to build and run it, the rules code has to follow, and what each crate does. |
| [docs/](docs/) | The glossary, architecture, performance and engineering rules, and the bridge contract. |

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
is translated yet. That work happens on the translation page at
[lumitlab.com](https://lumitlab.com), not in this repository — the only language
file edited here is the British-English source, and the others are written from
what the page sends back.

## Licence

[GPLv3](LICENSE). Forks stay open source.
