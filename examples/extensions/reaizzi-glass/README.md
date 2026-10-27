# Reaizzi Glass

Refractive glass for [Lumit](https://lumitlab.com/). A text, a shape, a logo or a
built-in shape becomes glass and bends everything beneath it, with a lit rim,
colour dispersion, frost and lens magnification.

It is an **LFX** effect: a shader Lumit runs on the GPU in its own pipeline, on
Windows, macOS and Linux. It installs into the official Lumit as an extension,
and works in the official Lumit on an adjustment layer. On the SICELIOT
build of Lumit it also works straight on the layer that gives it its shape.

## Installing

Double-click `reaizzi-glass.lfx`. Or, in Lumit, go to
**Settings ▸ Addons ▸ Extensions** and choose *Install from a folder* with this
folder. The effect appears in the Effects menu under **Distortion**.

## Using it

### On the layer itself (SICELIOT build of Lumit)

Apply **Reaizzi Glass** straight to a text, shape, solid, logo PNG or precomp.
That layer becomes the glass and bends everything beneath it; nothing has to be
picked. Its position, scale, rotation, animation and masks shape the glass, and
its opacity fades it. The layer's own pixels are not drawn, only the glass.

This needs the SICELIOT build of Lumit, whose engine reads the manifest's
`"backdrop": true`. The official Lumit ignores that line: there, use the
adjustment layer below.

Every other effect on that layer works on what is beneath it too, as on an
adjustment layer. To style the shape first (a stroke, a bevel), put the glass on
an adjustment layer instead, or precompose the styled layer.

### On an adjustment layer

1. Make an **adjustment layer** above everything the glass should refract.
2. Apply **Reaizzi Glass** to it.
3. In the **Shape layer** row, pick the layer that gives the glass its shape: a
   text, shape, solid, logo PNG or precomp. Turn its visibility off if you only
   want to see the glass; the effect still reads it.
4. Adjust Refraction, Depth, Dispersion and Light to taste.

The glass follows the shape layer's position, scale, rotation and animation.
The adjustment layer's masks and opacity limit where the glass shows, as they do
for any adjustment layer.

With no shape layer picked, the effect uses its built-in shape instead: a
rounded rectangle or an ellipse, with its own centre, size, corner radius and
rotation. You can also choose it directly under **Shape**.

## Controls

| Control | What it does |
|---|---|
| Shape | Shape layer, Rounded rectangle or Ellipse |
| Shape channel | Alpha, Luminance, or either one inverted |
| Centre X/Y, Width, Height, Corner radius, Shape rotation | The built-in shape |
| Refraction | How strongly the glass bends light at its edges |
| Depth | How far the distortion reaches inwards from the edge (px) |
| Dispersion | Splits red, green and blue at the edges, like a prism |
| Frost | Blurs what is seen through the glass, by this radius (px) |
| Splay | Pushes the distortion outwards, away from the centre |
| Magnify | Positive bulges like a lens, negative curves inwards |
| Magnify falloff | 100 = magnification ramps in from the edge, 0 = even |
| Effect scale | Scales depth, frost and rim together (0.1 to 4) |
| Edge highlight | A lit outline all round the glass, on any shape, text or layer. Off leaves clean glass with no outline |
| Rim width | Thickness of the lit outline (px) |
| Light angle | Where the light comes from (0° = from above, clockwise) |
| Light intensity | Brightness of the rim, 0 = off |
| Tint, Tint amount | Colours the glass |
| Mix | Dissolves back to the original picture (Lumit's standard row) |

On the SICELIOT build of Lumit, rows that do nothing as things stand are greyed: the
built-in shape's rows while a layer gives the shape, Shape channel while none does,
Corner radius on an ellipse, Rim width with Edge highlight off.

### Starting points

- **Prism**: Refraction 90, Depth 15, Dispersion 100.
- **Frosted**: Refraction 40, Frost 20, Light intensity 100.

## How it works

The glass is a height field: 0 on the edge, rising to 1 at the distance Depth
sets. For a built-in shape the height comes exactly from a distance function.
For a shape layer, the silhouette is blurred at a quarter of the resolution and
the blur stands in for the distance from the edge. The slope of the height gives
the direction of the refraction and of the light.

The lit outline comes from the full-resolution silhouette: the exact distance
for a built-in shape, a ring of samples round each pixel for a shape layer. So
a text's outline is as sharp as a built-in shape's.

The shader runs in six passes: the shape scaled down, two blurs of the shape,
two blurs for Frost, and the final composite. Every length is a pixel row,
which Lumit rescales whenever it renders at another resolution (a lowered
preview while dragging, for one), so the glass keeps its size and place at
every resolution and the preview matches the export.

## Limitations

- The glass is 2D: it follows the shape layer's 2D transform, not 3D
  perspective.
- With a shape layer, the refracting rim is only as sharp as the blurred
  silhouette, so hard corners round off slightly there. The glass's outline
  itself stays sharp.
- The *Shape layer* row is declared as a `depth` input, so that Lumit places it
  as it sits in the composition rather than stretching it over the frame. In
  the node graph it therefore shows as a Depth socket.
