---
title: Troubleshooting
description: Common problems.
sidebar:
  order: 3
---

## The app will not start

Lumit requires a GPU for rendering. You need a GPU that supports D3D12, Vulkan, or
Metal.

## Windows shows "Windows protected your PC"

The build is not code-signed on Windows, so SmartScreen warns on first run. Choose
**More info → Run anyway**. See [Installation](/start/install/).

## Media shows as missing

The project stores a reference to the file's path. If that file is moved or deleted, 
Lumit will be unable to find it and the asset reports it as missing.

To fix this, click the Relink button, and this allows you to locate the file to 
recover it. See [Importing media](/use/importing/).

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

## Related

- [Installation](/start/install/)
- [Preview and playback](/use/preview/)
- [Exporting](/use/export/)
