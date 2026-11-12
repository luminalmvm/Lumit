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
| **Interpretation** | How a footage item's file is read: its alpha, frame rate, fields, pixel aspect ratio, loop and colour. |
| **[Audio stream](/use/audio/#audio-streams)** | One of the sounds a media file carries side by side. |
| **[Packed project](/use/projects/#packed-projects)** | A project file that also carries its footage files inside it. |
| **Folder** | A grouping item in the Project panel. |
| **[Transcript](/use/captions/#transcripts)** | The words said in a footage item, each with when it is said and who says it. |
| **Composition** | A timeline holding layers. |
| **[Node graph](/use/node-graphs/)** (composition) | A composition whose picture is made by boxes and wires instead of a layer stack. |
| **[Multicam comp](/use/multicam/)** | A composition holding the cameras and recorders of one shoot, in step. |
| **Angle** | One camera or recorder in a multicam comp. A **sound angle** has no picture. |

## Layers and time

| Term | Meaning |
| --- | --- |
| **Layer** | One entry in a composition. |
| **Clip** | One item within a Sequence layer. |
| **[Caption layer](/use/captions/)** | A Text layer that shows the words being said, a few at a time. |
| **Multicam clip** | A clip that plays one angle of a multicam comp. |
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

## Sound

| Term | Meaning |
| --- | --- |
| **[Bus](/use/mixing/#buses)** | A layer that sums the sound routed to it and passes it on. |
| **Send** | A copy of a layer's sound taken to a bus at a level of its own. |
| **Gain** | A layer's or a clip's level before its effects and Volume. |
| **Volume** | A layer's fader, after its effects. |
| **Solo safe** | A layer or bus that stays heard whatever is soloed. |
| **[Automation mode](/use/mixing/#automation)** | How a strip's controls write keys while the composition plays. |
| **[Lower behind](/use/mixing/#lower-behind)** | A sound's level brought down while another is heard. |
| **[Loudness target](/use/mixing/#loudness)** | The loudness a composition's mix is made for. |
| **Stem** | One bus's sound, exported to a file of its own. |

## Related

- [Layers](/use/layers/)
- [Retiming and speed](/use/retime/)
