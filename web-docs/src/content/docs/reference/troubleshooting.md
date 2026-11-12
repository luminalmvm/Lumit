---
title: Troubleshooting
description: Common problems.
sidebar:
  order: 3
---

## The app will not start

Lumit requires a GPU for rendering. You need a GPU that supports D3D12, Vulkan, or
Metal.

When Lumit cannot finish starting, it opens a window headed **Lumit could not start**. The
window gives the reason and the path of `lumit-diagnostics.log`, the file the same line
was written to. Both can be selected and copied.

## A panel reads "This panel could not be drawn"

A panel that fails to draw is replaced by a box with that heading, the first line of the
fault, and the file it was recorded in. The other panels carry on.

## Windows shows "Windows protected your PC"

The build is not code-signed on Windows, so SmartScreen warns on first run. Choose
**More info → Run anyway**. See [Installation](/start/install/).

## Media shows as missing

The project stores a reference to the file's path. If that file is moved or deleted, 
Lumit will be unable to find it and the asset reports it as missing.

To fix this, click the Relink button, and this allows you to locate the file to 
recover it. See [Importing media](/use/importing/).

## The Viewer says the preview cannot be shown

On a Linux laptop with two graphics cards, the Viewer shows a message in place of the
picture when Lumit is rendering on the NVIDIA card and its window is drawn by the other
one.

- Switch the machine to NVIDIA Performance mode.
- Start Lumit with PRIME render offload.

## The Flatpak cannot open footage on a drive

The Flatpak reads the home folder and anything mounted under `/media`, `/run/media` and
`/mnt`. A folder anywhere else has to be granted. See
[Installation](/start/install/#linux).

## Importing video freezes the computer

Turn off **Settings ▸ Preview and cache ▸ Hardware video decoding** and open the project
again.

## There is no sound

**Settings ▸ Audio** names the output device. While a device chosen there is not
connected, Lumit plays through the system default and says so under the row. On a machine
with nothing to play through, the row reads **This machine has no audio output.**

## Playback is slow, or quality drops while working

Under load, or while scrubbing values to try and provide realtime previews, Lumit 
may reduce quality. For smoother playback, lower the preview resolution and let the
cache fill on a first pass. See [Preview and playback](/use/preview/).

Lumit is a compositor. If you have chosen to render every frame, Lumit will do so as
efficiently as possible; unfortunately you will be limited by your own hardware, but
where possible increase cache sizes to let Lumit reduce the need to render. Despite 
previews not always being realtime or taking time to render, Lumit aims to provide a
responsive interface regardless of what's rendering.

## Does exporting look the same as what I see in the Viewer?

It may not. Exporting allows you to change certain options, including resolution and 
frame rate. Despite this, export will always render at full quality. Any adjustments 
you've made to the Viewer, such as changing the preview resolution, exposure, displayed
grids, etc. are not seen by the export. See [Exporting](/use/export/).

## "Could not check for updates"

The update check got no answer it could use. Where GitHub refused the request, the
message names its answer instead, such as **GitHub answered 403**: GitHub answers 403 when
its hourly limit for the address is used up. The line under **Edit ▸ Settings ▸ General ▸
Updates ▸ This version** keeps the last reason. See [Installation](/start/install/).

## A plugin is missing

When the start-up scan turns a plugin away, Lumit writes the reasons to `plugin-scan.txt`,
beside `plugins.json` in Lumit's data folder:

| Platform | Folder |
| --- | --- |
| Windows | `%APPDATA%\Lumit\Lumit\data` |
| macOS | `~/Library/Application Support/dev.Lumit.Lumit` |
| Linux (Flatpak) | `~/.var/app/io.github.luminalmvm.Lumit/data/lumit` |

See [Plugins](/use/plugins/).

## Disk space drops while plugins are in use

Each OpenFX plugin bundle that renders keeps a frame file of about 512 MB,
`lumit-ofx-*.ring`, in the system temp folder. A bundle that is installed but not used
takes a few kilobytes. The files are deleted when Lumit closes.

## "Extensions need Microsoft WebView2, which isn't installed."

On Windows, extension panels need the Microsoft WebView2 runtime. **Get WebView2** opens
Microsoft's download page. See [Extensions](/use/extensions/).

## "This project is already open in another Lumit window"

A project is open in one Lumit window at a time. Opening it again brings that window
forward. Where the system does not allow that, this message shows instead, and the
project already loaded stays.

## Related

- [Installation](/start/install/)
- [Settings](/use/settings/)
- [Preview and playback](/use/preview/)
- [Exporting](/use/export/)
- [Plugins](/use/plugins/)
- [Extensions](/use/extensions/)
