---
title: Coming from After Effects, Vegas, Premiere Pro or Resolve
description: Coming from After Effects, Vegas, Premiere Pro and Resolve to Lumit.
sidebar:
  order: 3
---

Lumit is layer-based, in the manner of After Effects, with a few additions to help those 
coming from Vegas, Premiere Pro or Resolve, such as the Sequence layer, which is where 
cutting and speed ramping happen. Most of what you already know can be transferred.

On its first run Lumit asks which of the three you are coming from. Each answer sets a
few preferences, all of them ordinary rows in **Settings** afterwards.

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
Choosing **After Effects** when Lumit asks *How do you edit?* on its first run loads it.

Whole projects come across too. **File ▸ Import ▸ After Effects project**, and a 
report will appear telling you what was carried across with or without adjustments. 
This cannot port across third-party effects at this time, but it will still import the rest of 
a project which uses these.

Expressions written for After Effects run as written, and an animation preset (`.ffx`)
is applied from **Animation ▸ Apply animation preset**. See
[Expressions](/use/expressions/#what-javascript-can-read).

## Vegas to Lumit

| Vegas | Lumit |
| --- | --- |
| Event | **Clip**, inside a [Sequence layer](/use/sequence-layers/) |
| Track | **Layer**; the Sequence layer is the Vegas-style row you can cut on |
| Velocity envelope | **Retime**, edited through the Speed lens of the graph editor, or within a Sequence layer |
| Cursor | **Playhead** |
| Split | The **razor** tool, on the [toolbar](/panels/toolbar/) |

Choosing **Vegas** on the first run makes video arrive as a Sequence layer and opens the
Retime graph to speed.

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
