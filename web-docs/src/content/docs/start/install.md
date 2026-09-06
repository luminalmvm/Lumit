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

Download the `.exe` installer and run it.

Lumit is not signed on Windows. SmartScreen therefore shows a blue "Windows protected
your PC" panel the first time you run it, and you should choose **More info → Run
anyway**.

## Linux

Download the `.flatpak` and install it:

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

Download the `.dmg` installer and run it.

## Building from source

Clone the repository and follow the README. The engine is written in Rust and the
interface in Flutter.

```bash
git clone https://github.com/luminalmvm/Lumit.git
```

## Updating

Updates are installed automatically by default. If disabled Lumit can be updated from 
within the application via **Help ▸ Check for updates**, or in **Edit ▸ Settings ▸ 
General**. If these options don't work, you can manually update by downloading the 
[latest build](https://lumitlab.com/download).

A release that changes something a project may depend on says so after the download,
before Lumit restarts. **Not now** keeps the download and leaves the update in the menu.

Releases are announced on the [Lumit releases page](https://lumitlab.com/releases/), 
[GitHub releases page](https://github.com/luminalmvm/Lumit/releases), and our 
[Discord](https://discord.gg/dc3p3XC7mM).
