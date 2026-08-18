---
title: Coming from After Effects or Vegas
description: A term map from After Effects and Vegas to Lumit.
sidebar:
  order: 3
---

Lumit is layer-based like After Effects, with one extension: the Sequence layer, which
brings Vegas-style cutting. Most of what you know carries over; only some names change.

## After Effects to Lumit

| After Effects | Lumit |
| --- | --- |
| Time remapping | **Retime**, edited in the value graph of the [graph editor](/use/graph-editor/) |
| Track matte | [**Matte**](/use/mattes/), chosen from a dropdown on the layer |
| Pre-compose | **Precompose**; the result is a Precomp layer |
| CTI (current time indicator) | **Playhead** |
| Time stretch | **Stretch** |
| Null object | **Null layer** |

Keyframes use the same maths as After Effects - hold, linear, and bezier with speed and
influence - so your easing habits transfer directly. An After Effects keymap preset
ships in Settings for muscle-memory cases where the defaults deviate.

Importing an After Effects project is on the roadmap; it is not built yet.

## Vegas to Lumit

| Vegas | Lumit |
| --- | --- |
| Event | **Clip**, inside a [Sequence layer](/use/sequence-layers/) |
| Track | **Layer**; the Sequence layer is the Vegas-style row you cut on |
| Velocity envelope | **Retime**, edited through the Speed lens of the graph editor |
| Cursor | **Playhead** |
| Split | The **razor** tool, on the [toolbar](/panels/toolbar/) |

The value graph and the speed graph are two views of the same Retime data, so an After
Effects habit and a Vegas habit both land on the same feature.

Ready to start? Build your [first composition](/start/first-composition/).

## Related

- [Your first composition](/start/first-composition/)
- [Sequence layers](/use/sequence-layers/)
- [Retiming and speed](/use/retime/)
