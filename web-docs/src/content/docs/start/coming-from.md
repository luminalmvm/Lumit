---
title: Coming from After Effects, Vegas, Premiere Pro or Resolve
description: Coming from After Effects, Vegas, Premiere Pro and Resolve to Lumit.
sidebar:
  order: 3
---

Lumit is layer-based, in the manner of After Effects, with a few additions to help those 
coming from Vegas, Premiere Pro or Resolve, such as the Sequence layer, which is where 
cutting and speed ramping happen. Most of what you already know can be transferred.

On its first run Lumit asks which of the three you are coming from, under **Editing
style**. Each answer sets a few preferences, all of them ordinary rows in **Settings**
afterwards.

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
Choosing **After Effects** under **Editing style** on the first run, with
**Use the After Effects shortcuts** ticked, loads it.

Whole projects come across too. **File ▸ Import ▸ After Effects project…**, and a
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

Choosing **Vegas** on the first run opens the **Cut** workspace, makes video arrive as a
Sequence layer and opens the Retime graph to speed. **Only change default Retime graph** leaves video arriving as footage
layers. Tick **Use the Vegas shortcuts** to load those too. They are also the **Vegas**
preset in [Settings ▸ Shortcuts](/use/settings/).

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

Choosing **Premiere Pro or Resolve** on the first run makes footage arrive on Sequence
layers and opens Lumit in the **Cut** workspace: the Project panel, the Viewer with its
source view, and the Cut timeline. See [Cutting](/use/cutting/).

The next step is your [first composition](/start/first-composition/).

## Related

- [Your first composition](/start/first-composition/)
- [Sequence layers](/use/sequence-layers/)
- [Retiming and speed](/use/retime/)
- [Speed ramps, coming from Vegas](/tutorials/speed-ramps-from-vegas/)
