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
| **Project** | The whole document. Saved as a `.lum` file. One is open at a time. |
| **Asset** | Anything in the Project panel: footage, audio, images, compositions. |
| **Footage item** | An asset referencing a media file from disk. Lumit never changes the file. |
| **Interpretation** | How a footage item's file is read: its alpha, frame rate, fields, pixel aspect ratio, loop and colour. |
| **3D model** | A footage item referencing a glTF, GLB, FBX or OBJ file: its meshes, materials and pictures. |
| **File layer** | One layer of a layered image file, such as a PSD or an Illustrator document, which a footage item can read on its own. |
| **Audio item** | An asset referencing an audio file. |
| **[Audio stream](/use/audio/#audio-streams)** | One of the sounds a media file carries side by side. To **split** gives each a row of its own, to **merge** puts them back on one. |
| **[Packed project](/use/projects/#packed-projects)** | A project file that also carries its footage files inside it. The verbs are **pack** and **unpack**. |
| **[Shared project](/use/projects/#shared-projects)** | A project several people edit at once. The **host** shares it and the **guests** join with an **invite link**. One that is **kept online** is held by Lumit's server, so nobody's machine has to be on. |
| **Relay** | A server that passes a shared project's edits between a host and guests who cannot reach each other. It cannot read them. |
| **[Shared preview](/use/review-links/)** | An address where an export of a composition is watched in a browser and commented on. The window that lists them is **Previews**. The people watching are **reviewers**, and each upload is a **version**. |
| **Folder** | A grouping item in the Project panel. |
| **[Transcript](/use/captions/#transcripts)** | The words said in a footage item, each with when it is said and who says it. |
| **Comment** | A line of text kept on an asset. |
| **Composition** | A resolution, frame rate, duration and background, holding a layer stack or a node graph. |
| **[Node graph](/use/node-graphs/)** (composition) | A composition whose picture is made by boxes and wires instead of a layer stack. |
| **[Multicam comp](/use/multicam/)** | A composition holding the cameras and recorders of one shoot, in step. |
| **Angle** | One camera or recorder in a multicam comp. A **sound angle** has no picture. |

## Layers

| Term | Meaning |
| --- | --- |
| **Layer** | One row in a composition. The bottom layer draws first and each one above composites over it. |
| **Footage layer** | A layer showing one footage item. |
| **[Sequence layer](/use/sequence-layers/)** | Clips cut back to back on one row. |
| **Clip** | One item within a Sequence layer. A cut between two clips is an **edit point**. |
| **Multicam clip** | A clip that plays one angle of a multicam comp. |
| **Dissolve**, **transition** | Two clips overlapping on a picture layer: the later comes in as the earlier goes out, or a Transition effect runs over the earlier one. On an audio-only layer the overlap is a crossfade. |
| **Linked clips** | A picture clip and its sound clip, on two Sequence layers, that move, trim and cut together. |
| **Precomp layer** | A layer showing another composition. The verb is **precompose**, and the command is **Layer ▸ Precompose…**. |
| **Solid**, **Text**, **Shape**, **Null** layers | Flat colour, styled text, vector shapes, and a layer with a transform and no picture. |
| **[Caption layer](/use/captions/)** | A Text layer that shows the words being said, a few at a time. |
| **Adjustment layer** | A layer that applies its effects to everything below it. |
| **Layer group** | A header row folding layers together in the Timeline. |
| **Audio layer** | A layer playing an audio item, or footage's sound. |
| **Bus layer** | A layer that sound is routed to from other layers, through its own effects and Volume. A layer's **output** goes to the master or a bus, and a **send** copies its sound to a bus. |
| **Camera**, **Light** layers | A 3D viewpoint and a light. They affect 3D layers only. |
| **Model layer** | A 3D model in the composition's 3D space. Always 3D. |
| **Take** | One animation embedded in a 3D model's file. |
| **Part** | One node of a 3D model's file, listed under its Model layer. |
| **Card** | A flat Model layer with no 3D model behind it. |
| **3D shape** | A cube, sphere, cylinder, cone, torus, plane or pyramid Lumit makes itself, in the composition's 3D space. |
| **Geometry options** | A Text or Shape layer's extrusion depth and bevel, which make its outline a solid in the composition's 3D space. |
| **Projected from** | A material row naming the Camera layer whose view lays a picture on a Model layer. |
| **Anchor point** | The point a layer's transform pivots around. |
| **Orient** | Whether a layer turns by itself, along its motion path or towards the camera. |
| **Parenting** | A layer following another layer's transform. |
| **Switches** | A layer's toggles: visible, audible, solo, lock, shy, quality, motion blur, adjustment, 3D, collapse. |
| **Ripple** | An edit that moves everything after it by the same amount. **Insert** places a clip and ripples, **overwrite** places it over what is there. |
| **Roll**, **slip**, **slide** | Roll moves one edit point, slip changes which part of its source a clip shows, slide moves a clip between its neighbours. |
| **Lift**, **extract** | Lift removes a span and leaves the gap. Extract removes it and closes the gap. |
| **Match frame** | Loads a clip's footage into the source view on the frame the composition shows. |

## Animation and time

| Term | Meaning |
| --- | --- |
| **Property** | A named value that a layer's transform, effect, etc. has. Properties nest in **property groups**. |
| **Keyframe** | A property value linked to a specific frame, used to animate properties. Hold, linear or bezier. |
| **Graph editor** | Edits curves as a **value graph**, a **speed graph** or an **acceleration graph**. |
| **[Expression](/use/expressions/)** | A script on a property that computes its value each frame, written in JavaScript or Rhai. |
| **Custom controls** | An effect holding a named set of controls for expressions to read. |
| **[Marker](/use/markers/)** | A labelled point or span. **Beat markers** come from detecting beats in audio. |
| **Retime** | The one retiming system: a map from layer or clip time to source time. |
| **Speed** | How fast a value changes. On Retime, 100% is normal, 0% is a freeze and a negative speed runs backwards. |
| **Freeze** | A region of speed 0. |
| **Overrun** | Retime asking for time past the ends of the media. The end frame is held. |
| **Frame interpolation** | How in-between source frames are made: nearest, blend or flow. |
| **Stretch** | A command that rewrites the Retime map for a new speed. |
| **Motion blur** | The layer switch, the **Motion blur** effect, and **Accumulation motion blur**. |

## Picture

| Term | Meaning |
| --- | --- |
| **Mask** | A path on a layer that gates the layer's alpha. |
| **Matte** | Another layer's alpha or luma gating this one. |
| **Roto brush** | Builds a matte from painted strokes. **Refine edge** is its soft boundary. |
| **Blend mode** | How a layer composites over what is below. |
| **Effect** | One item in a layer's effect stack, or one box in a node graph. |
| **Source layer** | Another layer an effect reads as an input. |
| **Scene pass** | What a 3D precomp knows about its pixels, handed to effects: **Normals**, **Position**, **Part id** and **Depth**. |
| **Driver** | A box that makes a value rather than a picture and drives a parameter through a wire. |
| **Wire**, **port** | A connection on the Node graph panel, and the socket it plugs into. |
| **Points stream** | The points an effect produces for each frame. Never stored. |
| **Shapes** | Paths on a wire. A Read box hands out the paths of the shape layers in the composition it reads. |
| **Read box** | A box bringing a project item into a node graph. |
| **Input box** | A box standing for a value or a picture handed into a node graph from outside. |
| **Output box** | The one box whose picture a node graph shows. |
| **Merge** | The box laying one picture over another with a blend mode and an opacity. |
| **Switch box** | The box showing one of its pictures, chosen by an index. |
| **Time offset** | The box showing its input at another time. |
| **Split channels**, **Combine channels** | The boxes turning a picture into four greyscale pictures, and back. |
| **Repeat start**, **Repeat end** | The boxes that run what is wired between them several times, or once for each point or path. |
| **Node graph effect** | The effect that applies a node graph to a layer, with the graph's Inputs as its rows. |
| **Volumetric light** | A light seen in the air it shines through: its beam, the shafts 3D layers cut in it, its glow. |
| **Environment light** | A light from a picture of everything round the scene, which lights 3D layers and models and can show behind them. |
| **Working space** | The colour space pictures are composited in: scene-linear and premultiplied. |
| **OCIO config** | An OpenColorIO configuration a project can name. |

## Render, preview and export

| Term | Meaning |
| --- | --- |
| **Render** | The engine making pixels, for anything. |
| **Preview** | Playback inside Lumit. |
| **Export** | Renders and outputs a composition as a file. |
| **[Remote export](/use/remote-export/)** | An export one of an account's computers does for another. |
| **Cache** | Rendered frames kept in VRAM, RAM and on disk. |
| **[Proxy](/use/proxies/)** | A smaller copy of a footage file, kept beside the original and read in its place while working. |
| **Stand-in** | In a shared project, a small copy of a whole clip sent to someone who has not got the file. |
| **Preview resolution** | Full, Half, Third, Quarter or Auto. |
| **Adaptive degradation** | Lowering preview quality under load. It never touches an export. |

## Interface

| Term | Meaning |
| --- | --- |
| **Panel** | A dockable piece of the interface. |
| **[Workspace](/use/workspaces/)** | A saved arrangement of panels. |
| **[Stack](/use/workspaces/#stack-a-group)** | A group of panels drawn one above another, each under a header, instead of as tabs. |
| **[Profile](/use/profiles/)** | One person's settings on a machine: appearance, shortcuts, layout, preferences and what they have saved. |
| **Account** | A Lumit account, which a profile can be signed in to. Optional. |
| **Lumit Pro** | The paid plan on an account: Lumit's relay, sync, shared previews and backup. |
| **[Backup](/use/backup/)** | A copy of a project that Lumit Pro keeps each time it is saved. Its earlier saves are its **history**. |
| **Viewer** | Shows a composition, footage or a layer. Holds one, two or four **views**. |
| **Source view** | A Viewer view of one footage item, with **In** and **Out** marks. |
| **Timeline** | A composition's layers against time. |
| **Cut timeline** | The same layers as tracks of clips, for cutting. |
| **Node graph** panel | Draws a layer's effects as boxes and wires. |
| **Work area** | The span used for preview and for an export by default. |
| **Playhead** | The current-time marker. |
| **Scopes** | Waveform, RGB parade, vectorscope and histogram. |
| **Grade panel** | The selected layer's grade as wheels, primaries, curves and a look, over its Colour effects. |
| **Look** | A preset of Colour effects, picked from the Grade panel. It takes the place of the layer's grade. |
| **Easing panel** | One curve shape applied to selected keyframe spans. |
| **Retime panel** | The selected layer's speed, length, freeze, reverse and in-between frames. |
| **Text panel**, **Paragraph panel** | A text layer's font, size, spacing and outline, and how its lines are aligned and spaced. |
| **Expressions panel** | Keeps saved expressions and puts one on the selected properties. |
| **Flowchart** | A composition drawn between the compositions that place it and the ones it places. |

## Extending Lumit

| Term | Meaning |
| --- | --- |
| **[Plugin](/use/plugins/)** | An effect from somebody else. Lumit hosts **OFX** effects, and **CLAP** and **VST3** audio plugins. |
| **[LFX](/make/lfx-effects/)** | An effect an extension carries. |
| **Addon** | An optional download from Settings: the model runtime and the model packs. |
| **Model pack** | One addon holding one model. |
| **[Extension](/use/extensions/)** | A folder somebody else made that adds panels or effects. Its panel is shown in the dock. |
| **[Preset](/use/presets/)** | Saved effects, properties or animation. |
| **[Script](/use/scripts/)** | A JavaScript file that edits a project. After Effects `.jsx` scripts run as they are. |

## Sound

| Term | Meaning |
| --- | --- |
| **[Bus](/use/mixing/#buses)** | A layer that sums the sound routed to it and passes it on. |
| **Send** | A copy of a layer's sound taken to a bus at a level of its own. |
| **Gain** | A layer's or a clip's level before its effects and Volume. |
| **Volume** | A layer's fader, after its effects. |
| **[Solo safe](/use/mixing/#solo-and-mute)** | A layer that stays heard whatever is soloed. |
| **[Automation mode](/use/mixing/#automation)** | How a strip's controls write keys while the composition plays. |
| **[Lower behind](/use/mixing/#lower-behind)** | A sound's level brought down while another is heard. |
| **[Loudness target](/use/mixing/#loudness)** | The loudness a composition's mix is made for. |
| **Stem** | One bus's sound, exported to a file of its own. |

## Related

- [Layers](/use/layers/)
- [Retiming and speed](/use/retime/)
- [Keyboard shortcuts](/reference/shortcuts/)
