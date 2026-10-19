---
title: Installation
description: Install Lumit on Windows, Linux or macOS.
sidebar:
  order: 1
---

Lumit is free.

Download the build for your platform from
[lumitlab.com/download](https://lumitlab.com/download) to install.
If you would prefer to build the project from scratch, find it at [github.com/luminalmvm/Lumit](https://github.com/luminalmvm/Lumit).

## What you need

A GPU which has support for **Vulkan**, **Direct3D 12**, or **Metal** is required to run
Lumit. This can be either a graphics card or integrated graphics.

## Windows

Download the `.exe` installer and run it. It installs for the current user into
`%LOCALAPPDATA%\Programs\Lumit` with no administrator prompt, and carries the C++ runtime
Lumit needs.

Lumit is not signed on Windows. SmartScreen therefore shows a blue "Windows protected
your PC" panel the first time you run it, and you should choose **More info → Run
anyway**.

## Linux

Lumit for Linux is an x86-64 Flatpak. Download the `.flatpak` and install it:

```bash
flatpak install lumit-*.flatpak
```

Any Linux distribution with Flatpak works. If Flatpak is not set up on your
distribution, [flathub.org/setup](https://flathub.org/setup) covers it in a couple of
commands.

File associations for `.lum` and `.lumfx` are not created after installing the flatpak.
If you would like to be able to click on project files to open them directly, then clone
the repository from GitHub, and run `packaging/linux/install.sh`.

## macOS

Lumit runs on Macs with Apple silicon and macOS 12 or newer. Download the `.dmg` installer
and run it.

## Building from source

Clone the repository and follow the README. The engine is written in Rust and the
interface in Flutter.

```bash
git clone https://github.com/luminalmvm/Lumit.git
```

## First start

On its first start Lumit opens **Set up Lumit**. Each choice is applied as it is made,
and each is an ordinary row in **Settings** afterwards.

The **Editing style** cards set how footage arrives and how the Retime graph reads. See
[Coming from After Effects, Vegas, Premiere Pro or Resolve](/start/coming-from/).

| Card | Sets |
| --- | --- |
| **After Effects** | Footage arrives as a layer, and the Retime graph shows the time value of the source. **Use the After Effects shortcuts** loads that keymap. |
| **Vegas** | Video arrives as a Sequence layer, and the Retime graph shows speed as a percentage. **Only change default Retime graph** leaves video arriving as a layer. **Use the Vegas shortcuts** loads that keymap. |
| **Premiere Pro or Resolve** | Footage arrives on Sequence layers, in the Cut workspace. |

Below the cards are **Style**, **Colour scheme**, **Toolbar position** and **Motion**, and
the tick **Check for new versions of Lumit automatically**. **Skip** puts the look back as
it was. **Continue** keeps the choices.

### The tour

The guided tour runs once after setup, and again from **Help ▸ Lumit tour**. Each step
dims the window except one part and names it on a card: the Project panel, the Viewer,
the tools, the Timeline's layers, columns and properties, the graph editor, Effects &
presets, Effect controls, the effect menus, the workspaces and the node graph.

- **Next** and **Back**, or the Right and Left arrow keys, move between steps.
- **Skip** or `Esc` ends the tour. The last step's button is **Done**.
- A step whose part is not on screen is passed over.
- In an empty, unsaved project the tour builds a composition called **Tour** and removes
  it when it ends. It brings forward the panels it needs and puts the window back as it
  was.

## Updating

With **Edit ▸ Settings ▸ General ▸ Updates ▸ Automatic updates** on, Lumit checks for a
new version at launch, at most once a day. Downloading always waits for a click. The
[latest build](https://lumitlab.com/download) is also on the download page.

The **Help ▸ Check for updates** row and the button beside **This version** in the same
Settings group read what the updater is doing. The line under **This version** says
which version is installed and what the last check found.

| Label | Pressing it |
| --- | --- |
| **Check for updates** | Checks now. |
| **Click to update - v{version}** | Offers the update. |
| **Downloading update… {n}%** | Unavailable until the download ends. |
| **Restart to finish updating** | Asks about the restart. |

An update goes through these windows in turn:

1. **Update to Lumit {version}?**, with the file's name and size. **Not now** or
   **Download**.
2. The download's progress, with **Cancel**.
3. **Before you update to Lumit {version}**, only for a release whose notes have something
   to read first. **Not now** keeps the download and leaves the update in the menu.
   **Continue** goes on.
4. The restart: **Later** or **Restart now**. With unsaved changes the choices are
   **Later**, **Restart without saving** and **Save and restart**. **Later** keeps the
   download, and the next press comes straight back here.

On Windows the installer runs silently and Lumit comes back by itself. A Flatpak is
updated by Flatpak: the last window offers **Show the file**, and gives the
`flatpak install --user` command for the downloaded bundle.

Lumit keeps the file each update downloads, so the next update downloads only the parts
that changed. This applies to the Windows `.exe` and the macOS `.app.zip`. Where a patch
fails, the whole file is downloaded instead. Every download is checked against Lumit's signed
release manifest, and one that does not match is not installed. A Mac running a build
from before 0.8.0 is offered the `.dmg`.

### Nightly builds

**Settings ▸ General ▸ Updates ▸ Nightly builds** follows nightly builds instead of
releases. Turning it on asks first:

> **Use nightly builds?** New features first, less tested. Projects saved in a nightly
> may not open in the stable version. Keep a copy.

- A nightly is only ever offered nightlies.
- Turning the row off offers the latest stable release, even when it is older than the
  nightly installed.
- **Nightly icon**, on a nightly build on Windows and macOS, switches between the
  nightly's own icon and the regular one.
- A nightly build shows a **Nightly** pill on the status line.

Releases are announced on the [Lumit releases page](https://lumitlab.com/releases/), 
[GitHub releases page](https://github.com/luminalmvm/Lumit/releases), and our 
[Discord](https://discord.gg/dc3p3XC7mM).
