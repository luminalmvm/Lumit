<p align="right"><b>English</b> | <a href="./README.zh-CN.md">简体中文</a></p>

<p align="center">
  <a href="https://lumitlab.com">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="web/public/media/lumit-wordmark.svg">
      <img alt="Lumit" src="web/public/media/lumit-wordmark-dark.svg" width="300">
    </picture>
  </a>
</p>

<p align="center">
  <b>Compositing that adapts to your workflow.</b><br>
  A motion-graphics and compositing editor for Windows, macOS, and Linux. Built by editors, for editors.
</p>

<p align="center">
  <a href="https://github.com/luminalmvm/Lumit/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/luminalmvm/Lumit?sort=semver&label=release&style=flat-square&color=35785e"></a>
  <a href="https://github.com/luminalmvm/Lumit/releases"><img alt="Downloads" src="https://img.shields.io/github/downloads/luminalmvm/Lumit/total?style=flat-square&color=35785e"></a>
  <a href="https://github.com/luminalmvm/Lumit/actions/workflows/ci.yml"><img alt="CI" src="https://img.shields.io/github/actions/workflow/status/luminalmvm/Lumit/ci.yml?branch=main&label=CI&style=flat-square"></a>
  <a href="https://github.com/luminalmvm/Lumit/commits/main"><img alt="Commits a month" src="https://img.shields.io/github/commit-activity/m/luminalmvm/Lumit?style=flat-square&color=35785e"></a>
  <a href="LICENSE"><img alt="Licence: GPLv3" src="https://img.shields.io/badge/licence-GPLv3-35785e?style=flat-square"></a>
  <br>
  <img alt="Windows" src="https://img.shields.io/badge/Windows-1f2328?style=flat-square&logo=windows&logoColor=white">
  <img alt="macOS" src="https://img.shields.io/badge/macOS-1f2328?style=flat-square&logo=apple&logoColor=white">
  <img alt="Linux" src="https://img.shields.io/badge/Linux-1f2328?style=flat-square&logo=linux&logoColor=white">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-1f2328?style=flat-square&logo=rust&logoColor=white">
  <img alt="wgpu" src="https://img.shields.io/badge/wgpu-1f2328?style=flat-square&logo=webgpu&logoColor=white">
  <img alt="Flutter" src="https://img.shields.io/badge/Flutter-1f2328?style=flat-square&logo=flutter&logoColor=white">
</p>

<p align="center">
  <a href="https://lumitlab.com/download"><img alt="Download Lumit" src="https://img.shields.io/badge/Download_Lumit-35785e?style=for-the-badge&logo=data:image/svg%2Bxml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAxNiAxNiIgZmlsbD0ibm9uZSIgc3Ryb2tlPSJ3aGl0ZSIgc3Ryb2tlLXdpZHRoPSIxLjUiPjxwYXRoIGQ9Ik04IDJ2OU00IDdsNCA0IDQtNE0yIDEzaDEyIi8+PC9zdmc+" height="36"></a>
  <a href="https://discord.gg/dc3p3XC7mM"><img alt="Join the Discord" src="https://img.shields.io/badge/Join_the_Discord-5865F2?style=for-the-badge&logo=discord&logoColor=white" height="36"></a>
</p>

<p align="center">
  <a href="https://lumitlab.com">Website</a> ·
  <a href="https://docs.lumitlab.com">Manual</a> ·
  <a href="https://lumitlab.com/releases">Releases</a> ·
  <a href="https://lumitlab.com/projects">Example projects</a> ·
  <a href="https://lumitlab.com/translate">Translate</a> ·
  <a href="https://github.com/sponsors/luminalmvm">Sponsor</a>
</p>

<p align="center">
  <img src="assets/readme/hero.gif" alt="Lumit mid-edit: a burst of light playing in the Viewer, effect controls at the left, effects and presets at the right, and a Timeline of layers and keyframes below" width="100%">
</p>

> [!NOTE]
> Lumit is in public beta. If something breaks or is missing, please [open an issue](https://github.com/luminalmvm/Lumit/issues) or come and tell us on [Discord](https://discord.gg/dc3p3XC7mM).

## What Lumit is

Lumit brings the best of After Effects and Vegas into one editor, so you can composite, cut, and retime without leaving it. It started because most of my time frag editing in After Effects was spent waiting on previews. So the main goal is an editor that stays responsive, however many layers and keyframes a project has.

It's free and open source, forever. No subscription, no licence per seat, and no paying extra for the basics.

## Composite the way you know

The panels, shortcuts, and keyframes work the way After Effects users expect, and the Vegas style of retiming is one switch away. Projects from After Effects open in Lumit with their keyframes, base effects, and any OFX plugins Lumit can find.

<p align="center">
  <img src="assets/readme/lanes.gif" alt="Lumit's Timeline showing only the keyframed rows of several layers, with keyframes boxed and moved in the lanes" width="100%">
</p>

Effects can be driven by expressions, wired up in the node graph, or just set in the effect controls.

<p align="center">
  <img src="assets/readme/effects.gif" alt="Effect controls beside the Viewer: a Custom shader's own Wedges, Turn, Zoom, and Slice rows being dragged, and the kaleidoscope they fold the picture into" width="100%">
</p>

## What is in it

- **Retime, two ways.** Retime a layer on a value graph or a speed graph, After Effects or Vegas style, and switch between them at any point.
- **Cuts inside a layer.** A Sequence layer holds clips cut back to back on one row, like an NLE, and each clip keeps its own Retime.
- **Over 100 effects, built in.** Glow, motion blur, shake, RGB split, colour grading, Roto brush, Puppet, and many more. Every effect renders on the GPU and has its own page in the [manual](https://docs.lumitlab.com/effects/).
- **OFX, CLAP, and VST3 plugins.** Sapphire, Twixtor, and the rest are found at start-up and sit beside the built-in effects.
- **3D, text, and shapes.** 3D layers with a camera and depth of field, text layers with animators, and shape layers with paint.
- **Let music guide you.** Beat detection puts markers on the audio, cuts and retimes snap to them, and any property can be driven by a layer's sound.
- **Edit one project together.** One person hosts and up to 16 people can join, each with their own colour for their pointer, playhead, and selection.
- **Your work survives a crash.** Every edit is written to a journal as you make it, so if Lumit closes mid-edit your work is offered back next time.
- **The interface stays responsive.** A late frame lowers the preview quality instead of stalling, and you can keep working while an export runs.
- **Keep your shortcuts.** Load the After Effects keymap, or read in the shortcut file After Effects saves so your own binds carry over.
- **Scripting, the command line, and MCP.** Drive Lumit from a script, a terminal, or an AI tool.

## Workspaces

Each workspace arranges the window for one kind of work. Switch between them at any point.

<table>
  <tr>
    <td width="50%"><img src="web/src/assets/shots/graph-workspace.png" alt="The Graph workspace: effect controls, the Viewer, and a property's curve open in the Timeline"></td>
    <td width="50%"><img src="assets/readme/nodes.gif" alt="The Nodes workspace: a wired effect chain across the window, the Viewer beside it"></td>
  </tr>
  <tr>
    <td><b>Graph</b> - a property's curve, as a value graph or a speed graph.</td>
    <td><b>Nodes</b> - the same effects as a wired chain.</td>
  </tr>
  <tr>
    <td><img src="web/src/assets/shots/audio-workspace.png" alt="The Audio workspace: mixer strips at the left, levels and beat detection at the right, a waveform under the Timeline"></td>
    <td><img src="web/src/assets/shots/cut-workspace.png" alt="The Cut workspace: a clip's effects with their keyframes, a source view beside the Viewer, and tracks of clips with their sound under them"></td>
  </tr>
  <tr>
    <td><b>Audio</b> - mixer strips, levels, and beat detection.</td>
    <td><b>Cut</b> - clips on tracks, with a source view beside the Viewer.</td>
  </tr>
</table>

## Work together

<p align="center">
  <img src="assets/readme/shared.gif" alt="A shared project: two other people's pointers named in the Viewer, and their playheads in their own colours on the Timeline" width="100%">
</p>

Host a project and send an invite. Edits show up for everyone as they're made. [Lumit Pro](https://lumitlab.com/pro) adds a hosted relay for working together from anywhere, with previews, backup, and sync. The editor itself stays free.

## Download

Installers for Windows, macOS, and Linux (including Flatpak) are on [lumitlab.com/download](https://lumitlab.com/download) and the [latest GitHub release](https://github.com/luminalmvm/Lumit/releases/latest). Lumit checks for updates and installs them for you, or when you choose.

New to it? The manual starts with [installing Lumit and your first composition](https://docs.lumitlab.com/start/install/).

## Translations

Every translation comes from the community. Pick a language on the [translation page](https://lumitlab.com/translate), fill in what's missing, and send the file back as an issue.

<p>
  <a href="https://lumitlab.com/translate"><img alt="Chinese, Simplified" src="https://img.shields.io/endpoint?url=https%3A%2F%2Flumitlab.com%2Fbadges%2Fzh.json&style=flat-square"></a>
  <a href="https://lumitlab.com/translate"><img alt="Chinese, Traditional" src="https://img.shields.io/endpoint?url=https%3A%2F%2Flumitlab.com%2Fbadges%2Fzh_Hant.json&style=flat-square"></a>
  <a href="https://lumitlab.com/translate"><img alt="Spanish" src="https://img.shields.io/endpoint?url=https%3A%2F%2Flumitlab.com%2Fbadges%2Fes.json&style=flat-square"></a>
  <a href="https://lumitlab.com/translate"><img alt="German" src="https://img.shields.io/endpoint?url=https%3A%2F%2Flumitlab.com%2Fbadges%2Fde.json&style=flat-square"></a>
  <a href="https://lumitlab.com/translate"><img alt="Portuguese, Brazilian" src="https://img.shields.io/endpoint?url=https%3A%2F%2Flumitlab.com%2Fbadges%2Fpt.json&style=flat-square"></a>
  <a href="https://lumitlab.com/translate"><img alt="Italian" src="https://img.shields.io/endpoint?url=https%3A%2F%2Flumitlab.com%2Fbadges%2Fit.json&style=flat-square"></a>
  <a href="https://lumitlab.com/translate"><img alt="Polish" src="https://img.shields.io/endpoint?url=https%3A%2F%2Flumitlab.com%2Fbadges%2Fpl.json&style=flat-square"></a>
  <a href="https://lumitlab.com/translate"><img alt="Ukrainian" src="https://img.shields.io/endpoint?url=https%3A%2F%2Flumitlab.com%2Fbadges%2Fuk.json&style=flat-square"></a>
  <a href="https://lumitlab.com/translate"><img alt="Arabic" src="https://img.shields.io/endpoint?url=https%3A%2F%2Flumitlab.com%2Fbadges%2Far.json&style=flat-square"></a>
  <a href="https://lumitlab.com/translate"><img alt="Kazakh" src="https://img.shields.io/endpoint?url=https%3A%2F%2Flumitlab.com%2Fbadges%2Fkk.json&style=flat-square"></a>
</p>

## Building from source

The engine is Rust and wgpu, in `crates/`. The interface is Flutter, in `flutter_ui/`, and the two meet at `crates/lumit-bridge`.

You need Rust stable (pinned by `rust-toolchain.toml`), **FFmpeg 8.x** for media, and **LLVM 18** for the binding generator. It has to be FFmpeg 8, the build stops on 7 or 9. Newer LLVM silently generates broken bindings, so 18 is pinned on Windows and Linux.

<details>
<summary><b>Windows</b></summary>

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

The interface needs the Flutter SDK, see [flutter_ui/README.md](flutter_ui/README.md).

## The repository

- [docs/GUIDE.md](docs/GUIDE.md) is the place to start. How to build and run it, the rules code has to follow, and what each crate does.
- [docs/](docs/) has the glossary, architecture, performance and engineering rules, and the bridge contract.
- [crates/](crates/) is the engine, a Cargo workspace, and [flutter_ui/](flutter_ui/) is the interface.
- [web/](web/) and [web-docs/](web-docs/) are [lumitlab.com](https://lumitlab.com) and the [manual](https://docs.lumitlab.com), and depend on nothing else here.

## Contributing

Issues and pull requests are welcome. Read [docs/GUIDE.md](docs/GUIDE.md) first, and use the words in [docs/01-GLOSSARY.md](docs/01-GLOSSARY.md) in code, interface text, and commit messages. CI has to pass before anything is merged.

Translations don't happen in this repository. Use the [translation page](https://lumitlab.com/translate) instead, since the only language file edited here is the British English source.

## Support Lumit

[Sponsor on GitHub](https://github.com/sponsors/luminalmvm), get [Lumit Pro](https://lumitlab.com/pro), or just tell a friend.

## Licence

[GPLv3](LICENSE). Forks stay open source.
