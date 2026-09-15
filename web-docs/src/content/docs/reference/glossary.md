---
title: Glossary
description: Terms to use.
sidebar:
  order: 2
---

When trying to explain functionality, these are the terms used to refer to specific
areas and components of Lumit.

## Structure

| Term | Meaning |
| --- | --- |
| **Project** | The whole document. Saved as a `.lum` file. |
| **Asset** | Anything in the Project panel: footage, audio, images, compositions. |
| **Footage item** | An asset referencing a media file from disk. |
| **Folder** | A grouping item in the Project panel. |
| **Composition** | A timeline holding layers. |
| **[Node graph](/use/node-graphs/)** (composition) | A composition whose picture is made by boxes and wires instead of a layer stack. |

## Layers and time

| Term | Meaning |
| --- | --- |
| **Layer** | One entry in a composition. |
| **Clip** | One item within a Sequence layer. |
| **Property** | A named value that a layer's transform, effect, etc. has. |
| **Keyframe** | A property value linked to a specific frame, used to animate properties. |
| **Retime** | The adjusting of the default layer/clip time. |
| **Speed** | The rate a keyframed property's value changes at. |
| **Freeze** | A region of speed 0. |

## Picture

| Term | Meaning |
| --- | --- |
| **Mask** | A path on a layer that gates the layer's alpha. |
| **Matte** | Another layer used to gate the source layer's one. |
| **Blend mode** | How a layer composites over what is below. |
| **Effect** | One item in a layer's effect stack. |
| **Preview** | Playback inside Lumit. |
| **Export** | Renders and outputs a composition as a file. |
| **Playhead** | The current-time marker. |
| **Read box** | A box bringing a project item into a node graph. |
| **Input box** | A box standing for a value or a picture handed into a node graph from outside. |
| **Output box** | The one box whose picture a node graph shows. |
| **Merge** | The box laying one picture over another with a blend mode and an opacity. |
| **Switch box** | The box showing one of its pictures, chosen by an index. |
| **Time offset** | The box showing its input at another time. |
| **Node graph effect** | The effect that applies a node graph to a layer, with the graph's Inputs as its rows. |

## Related

- [Layers](/use/layers/)
- [Retiming and speed](/use/retime/)
