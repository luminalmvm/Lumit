What the next release has to say before anybody updates to it. A change that alters what a
project or a setting does adds its line here. Each nightly's GitHub release carries the
section below, and when the release note is written the lines move into it.

## Before you update

- Shake's Amplitude now scales Rotation amount and Z amount too, and at 0 the layer is left as it came in. Scale, Crop and Blend fade out over the last 4 pixels of Amplitude. A saved Shake renders as it did, unless its Amplitude is keyed down to 0, where its twist and zoom now settle with the fade. One with Amplitude set to 0 comes in at 30 with X and Y at 0, so it moves as it did. One with Amplitude scripted or wired keeps twisting until it's added again.
- A Precomp layer is no longer multiplied by its own alpha twice, so see-through parts and soft edges in a precomp match the same layers drawn flat, where they came out dark. Its blend modes, and the mattes and rows that read it, change with it. The trails of motion blurred layers lighten too.
- Particulate in the scene inside a collapsed 3D precomp now follows the precomp, where it used
  to stay behind when the precomp was moved.
- The camera's depth of field now blurs Model layers, which it used to leave sharp.
- Glow, and the other effects that spread light, no longer show a faint box behind each letter
  of a Text layer.
- Corner pin and Bezier warp in a node graph now draw past the picture they're handed, so a graph that pins its picture and then scales it may show what used to be cut off.
- An OpenEXR packed with PIZ is read by Lumit's own reader, as FFmpeg drew some with broken blocks, so those pictures can look different.
- A 3D model mirrored by a negative Scale has its normal map the right way round, so its bumps can look different.
- Objects hidden in Blender no longer come through from a `.blend`, which converts again.
- An environment light's Intensity stops at 0, where below it wrote negative pixels into a float export.
- The 3D layers and models in a collapsed precomp take the lights of the composition it sits in, where they ignored them.
- A collapsed precomp's own environment light no longer draws its picture flat in the parent.
- A layer with Accepts shadows on Only, or with Accepts lights off, now takes shadows with its back to the light, where it showed none. A floor at X rotation +90 can gain a shadow.
- An effect or material reading a Precomp layer now sees that layer's own paint and, with Effects and masks, its own effects, where it used to see the comp alone.
- A Precomp layer on Flow, as one from After Effects with Pixel Motion comes in, now makes the frames between, and is no longer collapsed.
