---
title: Coming from After Effects or Vegas
description: Coming from After Effects and Vegas to Lumit.
sidebar:
  order: 3
---

Lumit is layer-based, in the manner of After Effects, with a few additions to help those 
coming from Vegas, such as the Sequence layer, which is where Vegas-style cutting and 
speed ramping can happen. Most of what you already know can be transferred.

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

Whole projects come across too. **File ▸ Import ▸ After Effects project**, and a 
report will appear telling you what was carried across with or without adjustments. 
This cannot port across third-party effects at this time, but it will still import the rest of 
a project which uses these.

## Vegas to Lumit

| Vegas | Lumit |
| --- | --- |
| Event | **Clip**, inside a [Sequence layer](/use/sequence-layers/) |
| Track | **Layer**; the Sequence layer is the Vegas-style row you can cut on |
| Velocity envelope | **Retime**, edited through the Speed lens of the graph editor, or within a Sequence layer |
| Cursor | **Playhead** |
| Split | The **razor** tool, on the [toolbar](/panels/toolbar/) |

The next step is your [first composition](/start/first-composition/).

## Related

- [Your first composition](/start/first-composition/)
- [Sequence layers](/use/sequence-layers/)
- [Retiming and speed](/use/retime/)
