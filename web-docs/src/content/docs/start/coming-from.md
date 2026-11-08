---
title: Coming from After Effects, Vegas, Premiere Pro or Resolve
description: Coming from After Effects, Vegas, Premiere Pro and Resolve to Lumit.
sidebar:
  order: 3
---

Lumit is layer-based, in the manner of After Effects, with a few additions to help those 
coming from Vegas, Premiere Pro or Resolve, such as the Sequence layer, which is where 
cutting and speed ramping happen. Most of what you already know can be transferred.

On its first run Lumit asks for a workspace and a set of shortcuts. See
[First start](/start/install/#first-start).

## After Effects to Lumit

| After Effects | Lumit |
| --- | --- |
| Time remapping | **Retime**, edited in the value graph of the [graph editor](/use/graph-editor/) |
| Track matte | [**Matte**](/use/mattes/), chosen from a dropdown on the layer |
| Pre-compose | **Precompose**; the result is a comp layer |
| CTI (current time indicator) | **Playhead** |
| Time stretch | **Stretch** |
| Null object | **Null layer** |

Keyframes use the same maths as After Effects. Hold and linear are both there, as is
bezier with speed and influence. 

If you aren't used to all of Lumit's keybinds, an After Effects keymap 
preset ships in Settings, which you can access via **Edit ▸ Settings ▸ Shortcuts**.
Choosing **After Effects** under **Shortcuts** on the first run loads it.

Whole projects come across too. **File ▸ Import .aep project…**, and a
report will appear telling you what was carried across with or without adjustments.
Common third-party effects become the maker's own OFX plugin when it is installed. See
[Importing](/use/importing/).

Expressions written for After Effects run as written, and animation presets (`.ffx`) are
imported from **Settings ▸ General ▸ After Effects presets**. See
[Expressions](/use/expressions/#what-javascript-can-read).

## Vegas to Lumit

| Vegas | Lumit |
| --- | --- |
| Event | **Clip**, on a track of the [Cut timeline](/use/cutting/) |
| Track | A track of the Cut timeline, which is a [Sequence layer](/use/sequence-layers/) |
| Velocity envelope | The **speed line** across a clip. See [Speed ramps, coming from Vegas](/tutorials/speed-ramps-from-vegas/) |
| Playback rate | **Speed ▸ Speed…** on a clip's right-click menu |
| Cursor | **Playhead** |
| Split | `S` with the Vegas shortcuts, or the **razor** tool |
| Event FX | A clip's **fx** button and its own [effects](/use/cutting/#effects-and-blend-mode-on-a-clip) |
| Pan/Crop | A clip's **Pan/Crop** button: a [Crop](/effects/utility/crop/) and a Transform on the clip |
| Group, Ungroup | **Group** and **Ungroup**, `G` and `U` with the Vegas shortcuts |
| Auto ripple | **Auto ripple**, on the [Cut timeline](/use/cutting/#switches) |
| Automatic crossfades | **Auto crossfade**, on the Cut timeline |
| Quantize to frames | **Quantise**, on the Cut timeline |

Choosing **Vegas** under **Shortcuts** on the first run loads the **Vegas** preset from
[Settings ▸ Shortcuts](/use/settings/), opens the Retime graph to speed, and turns on
Vegas's clip drags and **Auto crossfade**. Choosing the **Cut** workspace as well makes
video arrive as a Sequence layer.

## Premiere Pro or Resolve to Lumit

| Premiere Pro, Resolve | Lumit |
| --- | --- |
| Sequence, timeline | **Composition** |
| Track | **Sequence layer**; picture on one, sound on another |
| Clip | **Clip**, inside a [Sequence layer](/use/sequence-layers/) |
| Bin | **Folder**, in the [Project panel](/panels/project/) |
| Source monitor | The Viewer's **source view**, with **In** and **Out** marks |
| Linked audio and video | **Linked clips**: a picture clip and its sound clip move as one |
| Ripple, roll, slip, slide | The same four edits, on the Cut timeline |

Choosing the **Cut** workspace on the first run opens the Project panel, the Viewer with
its source view, and the Cut timeline, and makes footage arrive on Sequence layers. See
[Cutting](/use/cutting/).

The next step is your [first composition](/start/first-composition/).

## Related

- [Your first composition](/start/first-composition/)
- [Sequence layers](/use/sequence-layers/)
- [Retiming and speed](/use/retime/)
- [Speed ramps, coming from Vegas](/tutorials/speed-ramps-from-vegas/)
