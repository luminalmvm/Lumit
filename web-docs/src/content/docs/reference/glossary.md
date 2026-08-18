---
title: Glossary
description: The words Lumit uses, and the ones it avoids.
sidebar:
  order: 2
---

Lumit is careful about words, because this field inherits three conflicting
vocabularies. One word means one thing here.

## Structure

| Term | Meaning |
| --- | --- |
| **Project** | The whole document. One is open at a time. Saved as a `.lum` file. |
| **Asset** | Anything in the Project panel: footage, audio, images, compositions. |
| **Footage item** | An asset referencing a media file on disk. |
| **Folder** | A grouping node in the Project panel. |
| **Composition** | A timeline holding an ordered stack of layers. |

## Layers and time

| Term | Meaning |
| --- | --- |
| **Layer** | One entry in a composition's stack. |
| **Clip** | One entry in a Sequence layer's row. Only means this there. |
| **Property** | A named animatable value. |
| **Keyframe** | A value anchored to a time. |
| **Retime** | The map from layer time (or clip time) to source time. |
| **Speed** | The rate the retime map runs at. |
| **Freeze** | A region of speed 0. |
| **Overrun** | A request for source time outside the media. |

## Picture

| Term | Meaning |
| --- | --- |
| **Mask** | A path on a layer that gates its own alpha. |
| **Matte** | Another layer used to gate this one. |
| **Blend mode** | How a layer composites over what is below. |
| **Effect** | One operation in a layer's effect stack. |
| **Preview** | Playback inside Lumit. Never writes files. |
| **Export** | Writing a deliverable file. |
| **Playhead** | The current-time marker. |

## Words we do not use

| Not this | This | Why |
| --- | --- | --- |
| Track | Layer | "Track" imports editing semantics that do not match a layer stack. |
| Velocity | Speed | Never shown; the graph editor’s derivative lens is labelled Speed. |
| Time remap | Retime | The After Effects name for one view of Retime. |
| Bin | Folder | A Premiere word. |
| CTI | Playhead | |
| Render *(meaning export)* | Export | Render is what the engine does, not what you do. |
| Event | Clip | A Vegas word. |
| Pre-render | Cache, or bake | Reserved for the engine's own cache warming. |

## Related

- [Layers](/use/layers/)
- [Retiming and speed](/use/retime/)
