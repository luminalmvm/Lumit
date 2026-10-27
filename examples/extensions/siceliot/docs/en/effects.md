# Siceliot effects and nodes

*Italiano: [../it/effetti.md](../it/effetti.md)*

What every Siceliot effect and node does, its parameters, an example of use, and what it
corresponds to in Cavalry. Names and parameter labels are written exactly as in Lumit. All of them
need the modified Lumit: see [../SICELIOT.md](../SICELIOT.md) for how to install it.

## Siceliot FX and Siceliot Nodes

SICELIOT files its work under two headings in Lumit.

| | **Siceliot FX** | **Siceliot Nodes** |
|---|---|---|
| What they are | Picture effects | Drivers |
| What they make | A picture: they change the layer's pixels | A number, which can change over time, per copy and per point |
| Where you find them | Add-effect menu, Effects & presets, node search | Only in the layer's Node graph (`Ctrl+Space`) |
| How you use them | Add them to a layer. They stack in order, each working on the picture of the one above, and their rows show in Effect controls | Wire one of their output sockets (**Value**…) into a parameter's socket. The parameter then follows the node instead of its keyframes |
| Mix, Blend, Matte rows | Yes | No: they draw nothing |
| Per copy | The same picture on every copy | Each copy of Clone to points, or each label of Label points, can get its own value |
| Built as | 19 shaders from the LFX extension **Siceliot** (`id: siceliot`, version 1.3.0, `extensions\siceliot`), and Type fill in the engine | In the engine (see [engine-changes.md](engine-changes.md)) |
| In Cavalry | Shaders, Filters and Deformers | Value Behaviours |

In short, an FX decides *what* you see and a node decides *how much*. A node can drive any
parameter of any effect, whether Siceliot's or Lumit's own. An FX row can be keyframed, set by an
expression or wired to a node.

## All effects and nodes

| Name | Heading | Cavalry equivalent | Match name |
|---|---|---|---|
| [Falloff](#falloff) | FX | Falloff (Circle, Rectangle, Linear, Sweep, Probability) | `lfx:siceliot/falloff` |
| [Conical gradient](#conical-gradient) | FX | Gradient Shader, Conical | `lfx:siceliot/conical-gradient` |
| [Sweep gradient](#sweep-gradient) | FX | Gradient Shader, Sweep | `lfx:siceliot/sweep-gradient` |
| [Multi-point gradient](#multi-point-gradient) | FX | Multi-Point Gradient Shader | `lfx:siceliot/multi-point-gradient` |
| [Gradient map](#gradient-map) | FX | Gradient Map Filter | `lfx:siceliot/gradient-map` |
| [Voronoi](#voronoi) | FX | Voronoi Shader | `lfx:siceliot/voronoi` |
| [Checkerboard](#checkerboard) | FX | Checkerboard Shader | `lfx:siceliot/checkerboard` |
| [Halftone](#halftone) | FX | Halftone Filter | `lfx:siceliot/halftone` |
| [Dithering](#dithering) | FX | Dithering Filter | `lfx:siceliot/dithering` |
| [Inner shadow](#inner-shadow) | FX | Inner Shadow Filter | `lfx:siceliot/inner-shadow` |
| [Erosion](#erosion) | FX | Erosion Filter | `lfx:siceliot/erosion` |
| [Light sweep](#light-sweep) | FX | Light Sweep Filter | `lfx:siceliot/light-sweep` |
| [Lighting](#lighting) | FX | Lighting Filter | `lfx:siceliot/lighting` |
| [Stripes](#stripes) | FX | Stripes Filter | `lfx:siceliot/stripes` |
| [Scrape](#scrape) | FX | Scrape Filter | `lfx:siceliot/scrape` |
| [Bilateral blur](#bilateral-blur) | FX | Bilateral Blur Filter | `lfx:siceliot/bilateral-blur` |
| [Bend](#bend) | FX | Bend Deformer (on pixels) | `lfx:siceliot/bend` |
| [Wave](#wave) | FX | Wave Deformer (on pixels) | `lfx:siceliot/wave` |
| [Four point warp](#four-point-warp) | FX | Four Point Warp (on pixels) | `lfx:siceliot/four-point-warp` |
| [Type fill](#type-fill) | FX (engine) | None in one node: the *Submesh Boolean* scene | `siceliot_type_fill` |
| [Stagger](#stagger) | Nodes | Stagger Behaviour | `siceliot_stagger` |
| [Random](#random) | Nodes | Random Behaviour | `siceliot_random` |
| [Oscillator](#oscillator) | Nodes | Oscillator Behaviour | `siceliot_oscillator` |
| [Value noise](#value-noise) | Nodes | Noise Behaviour | `siceliot_noise` |
| [Ring wave](#ring-wave) | Nodes | None | `siceliot_ring_wave` |
| [Clone to points](#clone-to-points) | Lumit's, extended | Duplicator | |
| [Label points](#label-points) | Lumit's, extended | Text Shapes on a Duplicator | |

## Rows every Siceliot FX effect also has

Lumit adds these to every effect, below its own rows, under **Compositing**:

| Row | What it does |
|---|---|
| **Mix** | How much of the result replaces the picture (0 to 100 %) |
| **Blend** | How the result is laid over the picture: Normal, Multiply, Screen, Add, Subtract, Overlay… |
| **Matte**, **Invert**, **Channel** | Another layer whose brightness decides where the effect acts |

Points are in **pixels at composition size**, measured from the top left of the layer. The
defaults are set for a 1920 × 1080 layer.

---

## Falloff

![Falloff](../images/falloff.png)

Draws *where* something should happen, from white (full) to black (none), as Cavalry's Falloff
does for its Behaviours. Use it three ways:

- **As a matte:** put Falloff on its own layer (Output *Greyscale*), hide that layer, and pick
  it as the **Matte** of any effect on another layer. The effect then acts only inside the
  falloff.
- **On its own layer:** Output *Fade layer* fades the layer's own picture by the falloff.
- **Stacked:** several Falloffs on one layer combine through their **Blend** row, as Cavalry's
  falloff layer modes do. *Multiply* keeps where both are, *Screen* or *Add* joins them,
  *Subtract* cuts one out of the other.

| Parameter | Control | Range | Default | Unit |
|---|---|---|---|---|
| **Shape** | Menu | Circle · Box · Linear · Sweep | Circle | |
| **Centre X**, **Centre Y** | Point | any | 960, 540 | px |
| **Width** | Slider | 0 to 4000 | 800 | px |
| **Height** | Slider | 0 to 4000 | 800 | px |
| **Rotation** | Dial | any angle | 0 | ° |
| **Softness** | Slider | 0 to 100 | 50 | % |
| **Arc** | Slider | 1 to 360 | 360 | ° |
| **Repetitions** | Whole number | 1 to 64 | 1 | |
| **Curve** | Menu | Linear · Smooth · Ease in · Ease out · Bell | Smooth | |
| **Strength** | Slider | 0 to 100 | 100 | % |
| **Invert falloff** | Checkbox | | Off | |
| **Binary** | Checkbox | | Off | |
| **Cell size** | Slider | 1 to 200 | 20 | px |
| **Seed** | Seed | any | | |
| **Output** | Menu | Greyscale · Alpha · Fade layer | Greyscale | |

- **Shape.** *Circle* and *Box* are full in their middle and fade out over **Softness** towards
  their edge (an ellipse or a rectangle Width × Height). *Linear* ramps from full to none across
  Width, along the direction set by Rotation. *Sweep* turns round the Centre from Rotation,
  clockwise, over **Arc**.
- **Repetitions** repeats the Linear and Sweep ramps inside their length.
- **Curve** reshapes the fall: *Bell* makes a ring, full halfway and none at the core and edge.
- **Strength** scales the result; **Invert falloff** swaps full and none (before Strength).
- **Binary** is Cavalry's *Probability*: the picture is cut into cells of **Cell size**, and each
  cell is fully on with a chance equal to the falloff there. **Seed** picks another pattern.
- **Output.** *Greyscale*: an opaque grey picture, for use as a matte. *Alpha*: white whose
  transparency is the falloff. *Fade layer*: the layer's own picture faded by the falloff.

**In Cavalry:** Falloff. There it is attached to a Behaviour or a Deformer and scales how much
it acts. Here it is a picture (grey or alpha), so it acts through a **Matte** and works on any
effect, Siceliot's or Lumit's.

**Example:** a Circle Falloff on a hidden layer, used as the Matte of a Gradient map on the
footage: only the middle of the picture is recoloured. Keyframe Width to open the circle.

---

## Conical gradient

![Conical gradient](../images/conical-gradient.png)

A ramp between two circles: the Start circle grows and slides into the End circle. With the two
centres on one spot it is a radial gradient; moving the Start centre off-centre gives the
off-centre "light inside a sphere" look. Animating **Start centre** and **Start radius** moves
a bright core through a shape, as in the Color Fusion style.

| Parameter | Control | Range | Default | Unit |
|---|---|---|---|---|
| **Start centre X**, **Start centre Y** | Point | any | 960, 540 | px |
| **Start radius** | Slider | 0 to 2000 | 0 | px |
| **End centre X**, **End centre Y** | Point | any | 960, 540 | px |
| **End radius** | Slider | 0 to 4000 | 800 | px |
| **Spread** | Menu | Pad · Repeat · Mirror | Pad | |
| **Paint** | Menu | Inside layer · Whole frame | Inside layer | |
| *Ramp rows* | | see [The ramp](#the-ramp) | | |

Where no circle of the cone reaches (when the Start circle sticks out of the End circle), the
result is clear.

**In Cavalry:** the Gradient Shader set to Conical. A Cavalry shader fills a shape; here
*Paint ▸ Inside layer* does the same on any layer. The ramp holds at most 5 colours.

**Example:** on a circle shape layer, keyframe Start centre from the top left to the middle and
Start radius from 0 to 200: a bright core slides into the sphere.

---

## Sweep gradient

![Sweep gradient](../images/sweep-gradient.png)

The ramp turns round a centre, like a radar or a colour wheel.

| Parameter | Control | Range | Default | Unit |
|---|---|---|---|---|
| **Centre X**, **Centre Y** | Point | any | 960, 540 | px |
| **Rotation** | Dial | any angle | 0 | ° |
| **Arc** | Slider | 1 to 360 | 360 | ° |
| **Repetitions** | Whole number | 1 to 64 | 1 | |
| **Spread** | Menu | Pad · Repeat · Mirror | Pad | |
| **Paint** | Menu | Inside layer · Whole frame | Inside layer | |
| *Ramp rows* | | see [The ramp](#the-ramp) | | |

The ramp starts at **Rotation** (0° points right) and runs clockwise over **Arc**.
**Repetitions** fits the ramp that many times into the Arc; with **Spread** *Mirror* the joins
are seamless.

**In Cavalry:** the Gradient Shader set to Sweep. Arc and Repetitions are built in, and the ramp
holds at most 5 colours.

**Example:** keyframe Rotation from 0° to 360° for a radar sweep. For seamless rays, set
Repetitions 6 and Spread *Mirror*.

---

## Multi-point gradient

![Multi-point gradient](../images/multi-point-gradient.png)

Up to six coloured points, each pulling the colours round it.

| Parameter | Control | Range | Default | Unit |
|---|---|---|---|---|
| **Points** | Whole number | 2 to 6 | 4 | |
| **Power** | Slider | 0.5 to 8 | 2 | |
| **Point 1 X** … **Point 6 Y** | Point | any | corners, centre, top | px |
| **Colour 1** … **Colour 6** | Colour | | orange, yellow, blue, purple, white, black | |
| **Influence 1** … **Influence 6** | Slider | 0 to 400 | 100 | % |
| **Paint** | Menu | Inside layer · Whole frame | Inside layer | |

- Only the first **Points** points are used.
- **Power** sets how sharply one point's colour gives way to the next: low values blend widely,
  high values make cells around each point.
- **Influence** makes a point reach further (above 100) or less far (below 100).

**In Cavalry:** the Multi-Point Gradient Shader. Here there are at most 6 points, and each point is
a pair of rows, not a separate object. To make a point follow a layer, wire or express its
position.

**Example:** keyframe the four points slowly round the frame for a moving mesh-gradient
background.

---

## Gradient map

![Gradient map after Falloff](../images/chain-falloff-gradient-map.png)

*Above: Falloff followed by Gradient map on the same layer.*

Reads how bright each pixel is and paints it from the ramp: dark from Position 0, bright from
Position 100. The layer keeps its own transparency.

| Parameter | Control | Range | Default | Unit |
|---|---|---|---|---|
| **Source** | Menu | Luminance · Red · Green · Blue · Alpha | Luminance | |
| **Reverse** | Checkbox | | Off | |
| *Ramp rows* | | see [The ramp](#the-ramp) | | |

Put it after any effect that makes greys (Falloff, Fractal noise, a blur…) to colour it. Its
default ramp turns black white, middle grey amber and white blue.

**In Cavalry:** the Gradient Map Filter. Here it holds at most 5 colours, and **Source** can read
one channel instead of the luminance.

**Example:** Lumit's Fractal noise followed by Gradient map turns grey noise into coloured smoke.

---

## Voronoi

![Voronoi](../images/voronoi.png)

A cellular pattern: one point scattered in each cell, and every pixel measured against the
nearest points.

| Parameter | Control | Range | Default | Unit |
|---|---|---|---|---|
| **Mode** | Menu | Cells · Distance · Edges | Cells | |
| **Cell size** | Slider | 4 to 1000 | 160 | px |
| **Randomness** | Slider | 0 to 100 | 100 | % |
| **Evolution** | Dial | any angle | 0 | ° |
| **Offset X**, **Offset Y** | Point | any | 0, 0 | px |
| **Edge width** | Slider | 0 to 100 | 4 | px |
| **Seed** | Seed | any | | |
| **Paint** | Menu | Inside layer · Whole frame | Inside layer | |
| *Ramp rows* | | see [The ramp](#the-ramp) | | |

- **Cells** paints each cell one colour picked from the ramp; **Distance** shades from each
  point (Position 0) outwards (Position 100); **Edges** draws the borders, ramp Position 0
  inside the cells and 100 on the borders.
- **Randomness** at 0 lines the points up in a grid.
- **Evolution** moves every point round a small circle: keyframe it from 0 to 360 for a
  seamless loop. **Offset** slides the whole pattern.

**In Cavalry:** the Voronoi Shader. Cavalry also has a Voronoi shape whose cells can be edited as
paths; this effect makes only pixels.

**Example:** Mode *Edges*, Cell size 200, with Evolution keyframed from 0° to 360° over the comp:
a cell net that loops.

---

## Checkerboard

![Checkerboard](../images/checkerboard.png)

| Parameter | Control | Range | Default | Unit |
|---|---|---|---|---|
| **Cell width** | Slider | 1 to 1000 | 120 | px |
| **Cell height** | Slider | 1 to 1000 | 120 | px |
| **Rotation** | Dial | any angle | 0 | ° |
| **Centre X**, **Centre Y** | Point | any | 960, 540 | px |
| **Colour 1** | Colour | | white | |
| **Colour 2** | Colour | | black | |
| **Paint** | Menu | Inside layer · Whole frame | Inside layer | |

A corner of four cells sits on the Centre. Edges are smoothed over one pixel, so the board
does not shimmer when it turns or moves.

**In Cavalry:** the Checkerboard Shader. Here the cells can be rectangles, and the board can be
turned.

**Example:** a test pattern under Bend, Wave or Four point warp, where you can see exactly how the
picture moves.

---

## Halftone

![Halftone on a Multi-point gradient](../images/halftone.png)

*Above: Multi-point gradient followed by Halftone.*

Redraws the picture as a printed screen. Each cell reads the picture once at its middle; the
darker it is, the bigger the mark.

| Parameter | Control | Range | Default | Unit |
|---|---|---|---|---|
| **Shape** | Menu | Dots · Lines · Squares | Dots | |
| **Cell size** | Slider | 2 to 200 | 16 | px |
| **Angle** | Dial | any angle | 45 | ° |
| **Softness** | Slider | 0 to 10 | 1 | px |
| **Keep colours** | Checkbox | | Off | |
| **Ink** | Colour | | black | |
| **Paper** | Colour | | white | |
| **Reverse** | Checkbox | | Off | |

- **Keep colours** prints each mark in the picture's own colour instead of **Ink**.
- **Reverse** makes bright areas get the big marks.
- The layer keeps its own transparency.

**In Cavalry:** the Halftone Filter. Here **Keep colours** prints each mark in the picture's own
colour.

**Example:** on footage, Cell size 10, Angle 15° and Keep colours on gives a comic-print look.

---

## Dithering

![Dithering on a Multi-point gradient](../images/dithering.png)

*Above: Multi-point gradient followed by Dithering, Colour, 2 levels.*

Cuts the picture down to a few levels, with the steps between them broken into a fine
pattern.

| Parameter | Control | Range | Default | Unit |
|---|---|---|---|---|
| **Method** | Menu | Ordered · Noise | Ordered | |
| **Levels** | Whole number | 2 to 16 | 2 | |
| **Pixel size** | Slider | 1 to 32 | 2 | px |
| **Colours** | Menu | Colour · Monochrome | Colour | |
| **Dark** | Colour | | near black | |
| **Light** | Colour | | warm white | |
| **Seed** | Seed | any | | |

- **Ordered** uses an 8 × 8 Bayer pattern, **Noise** a fixed random one (pick another with
  **Seed**).
- **Levels** per channel in *Colour*; in *Monochrome* the brightness is dithered between
  **Dark** and **Light**, which suits a retro screen look.
- **Pixel size** makes the dither pixels bigger.

**In Cavalry:** the Dithering Filter. Its patterns are an 8 × 8 Bayer or noise, in colour or in
two chosen tones.

**Example:** Colours *Monochrome*, Levels 2 and Pixel size 3, with Dark and Light set to two
greens, gives an old handheld-console screen.

---

## Inner shadow

![Inner shadow](../images/inner-shadow.png)

*Above: a 640 × 400 solid with Inner shadow at its defaults.*

A soft shadow cast inside the layer's edges.

| Parameter | Control | Range | Default | Unit |
|---|---|---|---|---|
| **Colour** | Colour | | black, 75 % opaque | |
| **Angle** | Dial | any angle | 45 | ° |
| **Distance** | Slider | 0 to 200 | 12 | px |
| **Size** | Slider | 0 to 200 | 16 | px |
| **Choke** | Slider | 0 to 100 | 0 | % |

- **Angle** is the direction the shadow falls (0° to the right, 90° down), so the default
  shades the top and left edges.
- **Size** is the blur; **Choke** makes the shadow denser before it fades.
- Past the layer's own edges counts as outside, so on a shape or text layer the shadow
  follows the shape.
- It is drawn in three passes, the two blurs at half size, so it stays light on modest
  graphics cards.

**In Cavalry:** the Inner Shadow Filter. The shadow stays inside the layer's own picture.

**Example:** on a text layer, Angle 90°, Distance 4 and Size 6: the letters look stamped into the
background.

---

## Erosion

![Erosion](../images/erosion.png)

*Above: Erosion at 8 px on a 640 × 400 solid: it becomes 624 × 384.*

Shrinks the layer's shape (a choke) or grows it (a spread).

| Parameter | Control | Range | Default | Unit |
|---|---|---|---|---|
| **Amount** | Slider | −64 to 64 | 8 | px |

- Above 0 each pixel takes the most transparent pixel within Amount: the shape shrinks.
  Below 0 it takes the most opaque one: the shape grows, carrying its edge colours out.
- It works across, then down, so corners stay square.
- Past the layer's edges counts as clear. A spread cannot grow past the layer's own
  picture; put the layer in a larger precomp first if it needs to.

**In Cavalry:** the Erosion Filter. Here it reaches at most 64 px either way, and a spread cannot
grow past the layer's own picture.

**Example:** keyframe Amount from 0 to 40 on a logo, and the shape is eaten away to nothing.

---

## Light sweep

![Light sweep over a Checkerboard](../images/light-sweep.png)

*Above: Checkerboard followed by Light sweep.*

A soft band of light across the layer. Keyframe **Centre** to sweep it over.

| Parameter | Control | Range | Default | Unit |
|---|---|---|---|---|
| **Centre X**, **Centre Y** | Point | any | 960, 540 | px |
| **Angle** | Dial | any angle | 30 | ° |
| **Width** | Slider | 0 to 2000 | 160 | px |
| **Softness** | Slider | 0 to 100 | 60 | % |
| **Colour** | Colour | | white | |
| **Intensity** | Slider | 0 to 400 | 100 | % |
| **Mode** | Menu | Add · Screen | Add | |

- **Angle** is the direction the band moves in when Centre moves along it; the band itself
  lies across it.
- **Softness** is how much of the Width fades at its sides.
- *Add* can push past white, for a hot glint; *Screen* never does.
- The light lands only where the layer draws.

**In Cavalry:** the Light Sweep Filter. The light falls only where the layer draws.

**Example:** on a logo, keyframe Centre X from −200 to 2100 over one second for a shine that
crosses it.

---

## Lighting

![Lighting](../images/lighting.png)

*Above: an alpha disc drawn by Falloff, then Lighting with a point light top left. The shadow
colour is blue here only because the comp is black.*

Long shadows and a light, as Cavalry's Lighting Filter. Every clear pixel looks towards the light
for something opaque in the way.

| Parameter | Control | Range | Default | Unit |
|---|---|---|---|---|
| **Light** | Menu | Point · Directional | Point | |
| **Light position X**, **Light position Y** | Point | any | 400, 200 | px |
| **Angle** | Dial | any angle | 45 | ° |
| **Length** | Slider | 0 to 2000 | 400 | px |
| **Fade** | Slider | 0 to 100 | 50 | % |
| **Shadow colour** | Colour | | black, 60 % opaque | |
| **Light colour** | Colour | | warm white | |
| **Light intensity** | Slider | 0 to 400 | 50 | % |
| **Light radius** | Slider | 0 to 4000 | 1200 | px |

- **Point** casts shadows away from **Light position**; **Directional** casts them all along
  **Angle**, like sunlight.
- **Length** is how far a shadow reaches, and **Fade** how much it thins on the way.
- The layer's own pixels are brightened by **Light colour** × **Light intensity**, less with
  distance up to **Light radius** (Point only).
- Shadows can only fall inside the layer's picture: use it on a layer as big as the comp, such
  as a precomp of the artwork.

**In Cavalry:** the Lighting Filter. Shadows can fall only inside the layer's picture, and blockers
are found with 48 samples.

**Example:** precompose some flat shapes, then add Lighting to the precomp with Light *Point* at
the top left. The shapes cast long shadows to the bottom right.

---

## Stripes

![Stripes on a Multi-point gradient](../images/stripes.png)

*Above: Multi-point gradient followed by Stripes at its defaults.*

Reads a band of pixels through **Centre** and repeats it along **Angle**. At a Width of 1 px
every pixel of the line is pulled out into a streak.

| Parameter | Control | Range | Default | Unit |
|---|---|---|---|---|
| **Centre X**, **Centre Y** | Point | any | 960, 540 | px |
| **Angle** | Dial | any angle | 0 | ° |
| **Width** | Slider | 1 to 1000 | 1 | px |
| **Side** | Menu | Both sides · Front · Back | Both sides | |

- **Angle** is the direction of the streaks (0° runs left and right).
- **Width** is the band repeated: wider bands make repeating slices of the picture.
- **Side** limits it to one side of the line: *Front* is the side Angle points to.

**In Cavalry:** the Stripes Filter. **Side** limits it to one side of the line.

**Example:** on footage with Width 1 and Side *Front*, keyframe Centre Y: the line of pixels
being stretched travels down the picture.

---

## Scrape

![Scrape on a Voronoi](../images/scrape.png)

*Above: Voronoi followed by Scrape, Centre moved left and Roughness at 60.*

Drags the pixels of a line through **Centre** out along **Angle**, fading as they go.

| Parameter | Control | Range | Default | Unit |
|---|---|---|---|---|
| **Centre X**, **Centre Y** | Point | any | 960, 540 | px |
| **Angle** | Dial | any angle | 0 | ° |
| **Length** | Slider | 0 to 4000 | 600 | px |
| **Roughness** | Slider | 0 to 100 | 30 | % |
| **Fade** | Slider | 0 to 100 | 50 | % |
| **Seed** | Seed | any | | |

- Only the side **Angle** points to is scraped; the rest is left alone.
- **Roughness** gives each row of pixels its own length, like paint pulled by a rough edge;
  **Seed** picks another set.
- **Fade** blends the drag back into the picture towards its end.

**In Cavalry:** the Scrape Filter. Only the side Angle points to is scraped, and **Roughness**
gives each row its own length.

**Example:** on a title, keyframe Centre X from left to right: the letters smear out behind it like
wet paint.

---

## Bilateral blur

| Before | After |
|---|---|
| ![Before](../images/bilateral-before.png) | ![After](../images/bilateral-blur.png) |

*Above: a noisy dithered gradient, then Bilateral blur at 20 px, Tolerance 45.*

Smooths areas of similar colour and keeps the edges between them sharp.

| Parameter | Control | Range | Default | Unit |
|---|---|---|---|---|
| **Radius** | Slider | 0 to 100 | 8 | px |
| **Tolerance** | Slider | 1 to 100 | 20 | % |

- A neighbour counts less the more its colour differs; **Tolerance** is how much difference
  still counts as "similar". Low keeps more detail, high blurs more.
- Drawn across, then down, 25 samples each.

**In Cavalry:** the Bilateral Blur Filter. Here it is drawn across and then down, which is close
to the full filter but not identical.

**Example:** clean up a noisy or dithered gradient before Halftone, or soften skin while keeping
the outlines.

---

## Bend

![Bend on a Checkerboard](../images/bend.png)

*Above: Checkerboard followed by Bend at its defaults.*

Curls the picture round a circle, as Cavalry's Bend Deformer does to a shape, but on pixels.

| Parameter | Control | Range | Default | Unit |
|---|---|---|---|---|
| **Centre X**, **Centre Y** | Point | any | 960, 540 | px |
| **Angle** | Dial | any angle | 0 | ° |
| **Bend** | Slider | −360 to 360 | 90 | ° |
| **Length** | Slider | 1 to 4000 | 600 | px |

- From **Centre**, along **Angle**, a stretch **Length** long is bent through **Bend**
  degrees. What lies past it carries on straight in the new direction; what lies behind Centre
  is left alone.
- Negative Bend curls the other way. Where nothing lands, the result is clear.

**In Cavalry:** the Bend Deformer. Cavalry bends a shape's points, so the outline stays sharp. Here
the pixels are moved: a shape layer is not redrawn sharp afterwards.

**Example:** on a long strip of text, Bend 180° and Length 400: the end of the strip curls back on
itself.

---

## Wave

![Wave on a Checkerboard](../images/wave.png)

*Above: Checkerboard followed by Wave at its defaults.*

A wave runs through the picture and pushes it sideways, like Cavalry's Wave on a path.

| Parameter | Control | Range | Default | Unit |
|---|---|---|---|---|
| **Shape** | Menu | Sine · Triangle · Square · Sawtooth | Sine | |
| **Amplitude** | Slider | 0 to 500 | 20 | px |
| **Wavelength** | Slider | 2 to 4000 | 200 | px |
| **Angle** | Dial | any angle | 0 | ° |
| **Phase** | Dial | any angle | 0 | ° |
| **Speed** | Slider | −10 to 10 | 1 | |

- **Angle** is the direction the wave runs; the picture is pushed across it.
- **Speed** is waves per second, so it moves on its own. For an exact loop set Speed to 0 and
  keyframe **Phase** from 0° to 360°.

**In Cavalry:** the Wave Deformer, which moves a path's points. Here the pixels are moved, and
**Speed** makes it move without keyframes.

**Example:** on a flag or a striped layer, Amplitude 30, Wavelength 400 and Speed 0.5.

---

## Four point warp

![Four point warp on a Voronoi](../images/four-point-warp.png)

*Above: Voronoi followed by Four point warp, two corners moved and two edges curved.*

Pins the layer's four corners to four points, and bows each edge in or out.

| Parameter | Control | Range | Default | Unit |
|---|---|---|---|---|
| **Top left X/Y** | Point | any | 0, 0 | px |
| **Top right X/Y** | Point | any | 1920, 0 | px |
| **Bottom right X/Y** | Point | any | 1920, 1080 | px |
| **Bottom left X/Y** | Point | any | 0, 1080 | px |
| **Curve top**, **Curve right**, **Curve bottom**, **Curve left** | Slider | −1000 to 1000 | 0 | px |

- The defaults match a 1920 × 1080 layer, so it starts with no change: drag the points in the
  Viewer.
- **Curve** bows an edge outwards (positive) or inwards (negative) at its middle, in place of
  Cavalry's bézier handles.
- Each pixel is traced back to the picture, so the result stays sharp. Outside the warped shape
  it is clear.

**In Cavalry:** Four Point Warp. Here each edge bows with one Curve instead of two bézier handles,
and the pixels are moved.

**Example:** pin a screen recording into the screen of a phone mock-up by dragging the four
corners onto the screen's corners.

---

## Type fill

![Type fill in the Flood example](../images/example-flood.png)

*Above: two Type fills on one Region layer, the Flood example (a giant G, Cut words on).*

Built into the engine (engine change 10), under **Siceliot FX**. Sets the **Words**, typed between
bars, in rows from top to bottom. Each row is cut into columns across the whole width, one word
each, as wide as the words naturally are. Along each row a column keeps only the stretches the
**Region layer** leaves free. The column's word is stretched across and down
from the first of them to the last, if that is at least **Min width** wide, and each stretch shows
its own slice of it: a shape that cuts a column in two cuts its word, "t" on one side and "he" on
the other, and never shows it twice. The look (font, style, colour) comes from a **Text layer**,
which can be hidden.

| Parameter | Control | Range | Default | Unit |
|---|---|---|---|---|
| **Text layer** | Layer | any | none | |
| **Words** | Text | words between `\|` | `Flood\|the\|Grid` | |
| **Region layer** | Layer | any | none (whole frame) | |
| **Region placement** | Menu | As placed in comp, Stretch to fit, Centre | As placed in comp | |
| **Region from** | Menu | Alpha, Luminance | Alpha | |
| **Invert region** | Checkbox | | off | |
| **Cut words** | Checkbox | | off | |
| **Threshold** | Slider | 0 to 100 | 50 | % |
| **Rows** | Slider | 1 to 200 | 24 | |
| **Row gap** | Slider | 0 to 200 | 6 | px |
| **Word gap** | Slider | 0 to 200 | 8 | px |
| **Min width** | Slider | 0 to 1000 | 24 | px |
| **Order** | Menu | In turn, Shift per row, Random | In turn | |
| **Seed** | Seed | any | random | |

- A place is in the region where the Region layer is at least **Threshold** opaque (Alpha) or
  bright (Luminance). **Invert region** fills where it is not.
- **Word gap** is the room between words and at the region's edges.
- **Cut words**: every word stays whole at its column's width and the region cuts it pixel by
  pixel, instead of stretching into the room it has. Two Type fills on both sides of a shape then
  share each word: "th" white inside, "e" black outside. Word gap then sets only the room between words.
- Either way the region cuts the words pixel by pixel, so a tall row never spills a word over a
  curved edge: no white word on white, no black word on black.
- **Order**: the same order in every row, moved on one word a row, or shuffled by **Seed**.
- Two Type fills on the same Region layer, one inverted and in the other colour, fill both
  sides of a shape. Put the shapes in a precomp to combine them with blend modes, and animate
  them: the words flood the room they leave.
- A condensed font (Impact, say) keeps the words readable when they are squeezed.

**In Cavalry:** there is no single node for it. Tim Siegert's *Submesh Boolean* scene builds the
look from text, a Duplicator and booleans. Here it is one effect, and the region can be any layer's
alpha or brightness.

**Example:** the [Flood](examples.md#flood) example: two Type fills on one precomp of animated
shapes, black words outside and white words inside.

---

## Siceliot Nodes

The five nodes are drivers: they make a number, not a picture (see
[Siceliot FX and Siceliot Nodes](#siceliot-fx-and-siceliot-nodes)). On every node, **Minimum**
and **Maximum** are sliders from −100 to 100, and you can type any value. Rows that the chosen mode
does not read are greyed out.

**How to use them.** Open the layer's **Node graph**, press `Ctrl+Space` (or right-click the
canvas) and pick the driver from the **Siceliot Nodes** heading. Drag a wire from its **Value** socket
to the socket of any parameter, of any effect, Siceliot or not. The parameter then follows the
driver instead of its keyframes.

**With Clone to points** (Lumit's Duplicator): put the driver on the layer being cloned. Each
copy then gets its own value, the way a Random or Stagger in Cavalry gives each copy of a
Duplicator its own. The layer is rendered once per copy (at most 32 different ones, Lumit's
own limit for Clone to points). Outside Clone to points a layer is the first copy of one, and
the drivers still work as plain values over time.

**Turning, sizing and fading each copy** (like Cavalry's Shape Rotation, Shape Scale and
Opacity on a Duplicator): put the driver on the **Points** layer, the one that carries Clone to
points, and wire its **Value** into Clone to points' own **Rotation**, **Scale** or
**Opacity**. Each stamp then takes its own value, turned and sized whole, with nothing cropped.
This is the way to rotate copies: a Transform effect on the cloned layer crops them, because a
layer's picture is only as big as its art.

| Without a driver | Stagger 0 → 90 into Rotation |
|---|---|
| ![](../images/copies-plain.png) | ![](../images/copies-stagger.png) |

Rotation is added to each point's own rotation, in degrees; Scale and Opacity are per cent, as
the rows are.

All five give the same value for the same frame every time, on every machine.

### Stagger

A value that steps from **Minimum** on the first copy to **Maximum** on the last.

| Parameter | Control | Range | Default |
|---|---|---|---|
| **Minimum** | Slider | −100 to 100, any by typing | 0 |
| **Maximum** | Slider | −100 to 100, any by typing | 100 |
| **Curve** | Curve | | straight diagonal |
| **Offset** | Slider | −100 to 100 % | 0 |
| **Loop** | Checkbox | | Off |
| **Reverse** | Checkbox | | Off |

- **Curve** spreads the values between the first copy and the last: an S-curve bunches them at
  both ends.
- **Offset** moves the run along the copies, as a share of them. Keyframe it, with **Loop** on,
  to send a wave through the copies.
- **Reverse** gives Maximum to the first copy.

**In Cavalry:** the Stagger Behaviour, with its graph as **Curve**.

**Example:** Stagger from 0 to 90 into Clone to points' Rotation (the pictures above). Keyframe
Offset from 0 to 100 % with Loop on, and a turn runs through the copies.

### Random

A random value for each copy, the same every frame.

| Parameter | Control | Range | Default |
|---|---|---|---|
| **Minimum** | Slider | −100 to 100, any by typing | 0 |
| **Maximum** | Slider | −100 to 100, any by typing | 100 |
| **Bias** | Slider | −100 to 100 | 0 |
| **Rate** | Slider | 0 to 30 | 0 |
| **Seed** | Seed | any | |

- **Bias** leans the values towards Minimum (below 0) or Maximum (above 0).
- **Rate** is new values a second; at 0 each copy keeps one value for good.
- **Seed** picks another set of values.

**In Cavalry:** the Random Behaviour. Here **Bias** leans the values, and **Rate** gives new ones
over time without keyframes.

**Example:** Random from 40 to 160 into Clone to points' Scale: every copy gets its own size.
With Rate 4 they change size four times a second.

### Oscillator

A value that swings between **Minimum** and **Maximum** in a wave.

| Parameter | Control | Range | Default |
|---|---|---|---|
| **Shape** | Menu | Sine · Triangle · Square · Sawtooth | Sine |
| **Minimum** | Slider | −100 to 100, any by typing | 0 |
| **Maximum** | Slider | −100 to 100, any by typing | 100 |
| **Frequency** | Slider | 0 to 20 | 1 |
| **Phase** | Dial | any angle | 0° |
| **Phase per copy** | Dial | any angle | 0° |
| **Phase from** | Menu | Copy · Position | Copy |
| **Wave angle** | Dial | any angle | 0° |
| **Wavelength** | Slider | 10 to 4000 | 400 px |

- **Frequency** is waves a second. For an exact loop set it to 0 and keyframe **Phase** from
  0° to 360°.
- **Phase from = Copy**: **Phase per copy** puts each copy that much further along the wave than
  the one before, so the copies ripple in the order they are numbered (a Grid: row by row).
- **Phase from = Position**: each copy's place on the wave comes from **where it stands**. The
  wave crosses the points like a texture, travelling along **Wave angle** (0° left to right, 90°
  top to bottom), one crest every **Wavelength** pixels. It works on any arrangement of points,
  Grid, Scatter or Particulate alike. Position is known only when the driver is wired into
  Clone to points' own Scale, Rotation or Opacity; anywhere else it reads as the comp's top-left
  corner.
- A sine starts halfway between Minimum and Maximum, on its way up.

![Oscillator by Position into Scale](../images/copies-wave-0.png)

*Above: a 24 × 13 Grid of bars, an Oscillator with Phase from = Position (Wave angle 30°,
Wavelength 600 px, Minimum 100, Maximum 900) wired into Clone to points' Scale. Frame 12, half a second later: [copies-wave-12.png](../images/copies-wave-12.png).*

**In Cavalry:** the Oscillator Behaviour. Here it makes values only; to make the picture itself
wave, use the [Wave](#wave) effect. **Phase from = Position** is SICELIOT's own: a wave that
crosses the points in space.

**Example:** the picture above. With Phase from *Copy* and Phase per copy 15°, the copies pulse
one after another in their own order.

### Value noise

A value that wanders smoothly, as Cavalry's Noise behaviour. (Lumit's own **Noise** is a grain
effect, hence the different name.)

| Parameter | Control | Range | Default |
|---|---|---|---|
| **Minimum** | Slider | −100 to 100, any by typing | 0 |
| **Maximum** | Slider | −100 to 100, any by typing | 100 |
| **Frequency** | Slider | 0 to 20 | 1 |
| **Detail** | Whole number | 1 to 4 | 1 |
| **Vary by** | Menu | Nothing · Copy · Position | Copy |
| **Noise scale** | Slider | 10 to 4000 | 300 px |
| **Seed** | Seed | any | |

- **Frequency** is how quickly it wanders. **Detail** adds finer wander on top, each layer half
  as strong.
- Rows the chosen mode does not read are greyed out: Phase per copy on Position, Wave angle and
  Wavelength on Copy (Oscillator), Noise scale unless Vary by is Position (Value noise).
- **Vary by**: *Nothing*, every copy moves together; *Copy*, every copy wanders on its own path;
  *Position*, the copies read a field of noise laid over the points, so neighbours move alike
  and the pattern drifts as time goes on. **Noise scale** is the size of the field's features.
  As with Oscillator, Position is known when the driver is wired into Clone to points' own rows.

**In Cavalry:** the Noise Behaviour. It has another name here because Lumit's **Noise** is a grain
effect. *Vary by ▸ Position* gives a field of noise across the points.

**Example:** Vary by *Position*, Noise scale 200 and Minimum −20 to Maximum 20 into Clone to
points' Rotation: the copies sway together, as grass does in the wind.

### Ring wave

Rings that grow out one after another from points on the frame, like stones dropped in water
(engine change 7). It is Lumit's **Ripple** as a value rather than a distortion, hence the
different name. Each point reads how strong the rings are on it, so put it on the **Points**
layer and wire it into Clone to points' or Label points' own rows.

| Parameter | Control | Range | Default |
|---|---|---|---|
| **Minimum** | Slider | −100 to 100, any by typing | 0 |
| **Maximum** | Slider | −100 to 100, any by typing | 100 |
| **Interval** | Slider | 0.05 to 5 s | 0.5 |
| **Speed** | Slider | 10 to 2000 px a second | 300 |
| **Speed variance** | Slider | 0 to 500 px a second | 10 |
| **Thickness min** | Slider | 1 to 1000 px | 60 |
| **Thickness max** | Slider | 1 to 1000 px | 120 |
| **Origin** | Menu | Anywhere · Edges · Centre · Away from points | Anywhere |
| **Clearance** | Slider | 0 to 400 px | 20 |
| **Add up** | Checkbox | on or off | off |
| **Grow min** | Slider | 0 to 200 % | 20 |
| **Grow max** | Slider | 0 to 200 % | 50 |
| **Spin** | Slider | 0 to 200 % | 100 |
| **Seed** | Seed | any | |

- A new ring starts every **Interval** seconds, at a random place on the frame (*Anywhere*), on
  its border (*Edges*), always in the middle (*Centre*), or at a random place at least
  **Clearance** away from every point it is wired to (*Away from points*: a ring never starts
  inside the shape the points fill, as in the Processing sketch). It grows at **Speed**, each
  ring up to **Speed variance** faster or slower. **Seed** decides where and how fast. Clearance
  is greyed out on the other origins.
- **Thickness min/max**: how far from its middle line a ring fades to nothing; each ring is
  somewhere between the two.
- **Add up**: off, rings that overlap add up to Maximum at most; on, they keep adding past it,
  as the sketch's do.
- It has four output sockets:
  - **Value**: Minimum where no ring is, Maximum on a ring's middle line, smoothly in between.
  - **Rotation**: a turn in degrees that a ring gives the points it crosses, strongest on its
    middle, following the direction the ring travels and spinning slowly. **Spin** scales it.
  - **Passes**: how many rings have crossed the point so far. It goes up by one as each ring's
    middle reaches the point, and never goes back.
  - **Grow**: how much bigger the rings make the point, per cent. Each ring has its own amount,
    between **Grow min** and **Grow max**, given in full on its middle line; overlapping rings
    add up. Wire it, plus 100 through a **Math** (Add), into Label points' **Label scale**.
- Like the other drivers it has no memory: every frame is worked out from the time alone, so
  scrubbing, rendering out of order and rendering again all give the same picture.

**In Cavalry:** none. The nearest thing is a Circle Falloff with a keyframed radius on a
Behaviour, one ring at a time. Lumit's own **Ripple** distorts pixels instead.

**Example:** the [Sketch](examples.md#sketch) example. Ring wave's Value goes into Label points'
Change, its Passes into Text step and its Grow (plus 100) into Label scale.

## Lumit effects SICELIOT extends

Two of Lumit's own point effects (*Effect ▸ Generate*) get new rows from the engine changes. They
stay under Lumit's heading.

### Clone to points

Stamps a layer on every point, as Cavalry's Duplicator does. SICELIOT adds the per-copy values
described under [Siceliot Nodes](#siceliot-nodes):

- a node wired into its own **Scale**, **Rotation** or **Opacity** gives each stamp its own value;
- a node on the cloned layer renders that layer once per copy.

**In Cavalry:** the Duplicator, with Shape Rotation, Shape Scale and Opacity driven by Behaviours.
A layer that differs per copy is rendered at most 32 different ways, which is Lumit's limit.

### Label points

Writes a label at every point, with the look of a Text layer. SICELIOT adds random text, labels
that react to a node, smooth colours, and a size and turn for each label.

**Setting it up**

1. Draw a **Text layer** with the font, size and colour you want, and hide it: it only lends its
   look.
2. On the **Points** layer, after **Grid** (or any points producer), add *Effect ▸ Generate ▸
   **Label points***. It wires itself to the producer above. Turn **Clone to points** off, or
   keep both for text beside shapes.
3. Set **Text layer** to your Text layer, **Anchor** to Centre and **Offset x / y** to 0, so each
   label sits on its point.
4. Type a template in **Text**. Random tokens mix with ordinary text and with Lumit's own tokens,
   for example `ID-{index}-{chars:4}`:

| Token | Written as | Example |
|---|---|---|
| `{pick:a\|b\|c}` | one of the words between the bars, per point | `{pick:SUN\|MOON\|STAR}` |
| `{chars:N}` | N random letters and digits | `{chars:6}` → `aZ3kQ9` |
| `{hex:N}` | N random hexadecimal digits | `#{hex:6}` → `#3fa21c` |

Each point keeps its text on every frame, whatever order the points come in, and **Seed** rolls a
new set. The Text row holds 128 characters. For a longer word list, leave it empty and type the
template in the Text layer itself.

**Rows SICELIOT adds**

| Parameter | Control | Range | Default | Unit |
|---|---|---|---|---|
| **Change** | Slider | 0 to 100 | 0 | % |
| **Change above** | Slider | 0 to 100 | 50 | % |
| **Scramble rate** | Slider | 0 to 60 | 12 | per second |
| **Text step** | Slider | 0 to 1000, any by typing | 0 | |
| **Flash colour** | Menu | Off · Random hue · Two colours · By strength · Gradient | Off | |
| **Colour A**, **Colour B** | Colour | | red, blue | |
| **Hue start** | Dial | any | 0° | |
| **Hue range** | Slider | 0 to 360 | 200 | degrees |
| **Saturation** | Slider | 0 to 100 | 80 | % |
| **Darken** | Slider | 0 to 100 | 50 | % |
| **Gradient stops** | Whole number | 2 to 4 | 4 | |
| **Gradient 1** … **Gradient 4** | Colour | | white, sand, green, blue | |
| **Label scale** | Slider | 0 to 400 | 100 | % |
| **Grow** | Slider | 0 to 200 | 0 | % |
| **Label rotation** | Dial | any | 0° | |
| **Vertical anchor** | Menu | Middle · Top · Bottom | Middle | |
| **Seed** | Seed | any | | |

- **Change** says how strongly something acts on each label. Wire a node into it (an Oscillator,
  a Ring wave) and each label reads its own value. Where Change is above **Change above**, the
  label is *in the wave*: its random tokens roll a new text **Scramble rate** times a second. At 0,
  a label shows just one other text while the wave is on it. Once the wave has passed, the label
  goes back to its resting text.
- **Text step**: each whole number gives the random tokens a new roll, and the roll stays. Wire
  Ring wave's **Passes** into it, and the character changes as each ring goes by and keeps the
  new one.
- **Flash colour** colours the labels:
  - *Random hue* flashes any hue while the label is in the wave.
  - *Two colours* flashes Colour A or Colour B at random.
  - *By strength* follows Change smoothly. At 0 the label is the Text layer's colour. As Change
    grows, the hue runs from **Hue start** over **Hue range**, the colour rises to **Saturation**,
    and the label darkens by up to **Darken**. Past 100 the hue keeps turning.
  - *Gradient* is the same smooth colouring through your own colours. **Gradient 1** is the colour
    at Change 0 and the last of the **Gradient stops** is the colour at 100. The colours are mixed
    in linear light.

  The colouring multiplies the Text layer's colour, so with white text you see exactly the colour
  that is set.
- **Label scale** sizes each label, and **Grow** adds how much bigger a label is at Change 100.
  Clone to points' per-copy Scale and Rotation do not reach labels.
- **Label rotation** turns each label about its anchor, clockwise. Wired to a node, each label
  turns its own way.
- **Vertical anchor** chooses which part of the label's height sits on its point, and what the
  label turns and grows about. *Top* hangs the letter from its point.
- **Max labels** (default 300) caps how many labels are drawn. Raise it for big grids.
- Labels are drawn between pixels, at any size and any turn, so they never jump from pixel to
  pixel or grow in steps.

**In Cavalry:** Text Shapes on a Duplicator, with a String Generator for random text and
Behaviours on the attributes. Here it is one effect on a Points layer, the look comes from a
hidden Text layer, and scrambling, colour and size all follow one number, Change.

**Example: decode.** On a 24 × 13 Grid with Text `{pick:A|K|M|R|S|Z|7|4}`:

1. Add an **Oscillator** with Phase from *Position*, Minimum 0 and Maximum 100, and wire it into
   **Change**.
2. Set Flash colour to *Random hue*.
3. Wire a second Oscillator (Minimum 70, Maximum 170) into **Label scale**.

A wave of scrambling, flashing letters crosses the grid.

![Decode](../images/labels-wave-0.png)

*Above: Wave angle 30°, Wavelength 600 px, Max labels 400.*

**Example: letters inside a letter.** The [Sketch](examples.md#sketch) example, built step by step
there.

| By strength | Gradient (white, orange, magenta, violet) |
|---|---|
| ![Rings](../images/sketch-rings.png) | ![Gradient](../images/sketch-gradient.png) |

---

## The ramp

Conical gradient, Sweep gradient, Gradient map and Voronoi share the same ramp rows:

| Parameter | Control | Range | Default |
|---|---|---|---|
| **Interpolation** | Menu | Linear · Smooth · Constant | Linear |
| **Stops** | Whole number | 2 to 5 | 3 |
| **Colour 1**, **Position 1** | Colour, slider 0 to 100 % | | white, 0 |
| **Colour 2**, **Position 2** | | | amber, 50 |
| **Colour 3**, **Position 3** | | | blue, 100 |
| **Colour 4**, **Position 4** | | | magenta, 100 |
| **Colour 5**, **Position 5** | | | black, 100 |

- Only the first **Stops** colours are used. They are sorted by Position, so the rows can be in
  any order.
- *Smooth* eases between stops; *Constant* holds each colour until the next stop, for hard
  bands.
- **Spread** (Conical and Sweep): *Pad* holds the end colours, *Repeat* starts the ramp again,
  *Mirror* runs it back.
- **Paint** (all gradients): *Inside layer* paints only where the layer draws, as a Cavalry fill
  shader; *Whole frame* fills the layer's whole picture.

Every row can be keyframed, driven by an expression or wired to a driver in the node graph.

---

## Known limits

- These effects install only in a Lumit built with the [Siceliot FX heading](engine-changes.md).
- Lumit's scripting (`lumit.effect.set`) cannot set an LFX effect's own rows, only Mix, Blend
  and the matte. Set them in Effect controls, by keyframes or by expression.
- The ramp holds at most 5 colours and Multi-point gradient at most 6 points: an LFX row list is
  fixed when the shader is written.
- Erosion reaches at most 64 px, and Inner shadow blurs with 25 samples per direction, so a very
  large Size looks slightly stepped.
- An LFX effect cannot draw past its layer's own picture: Erosion's spread, Inner shadow and
  Lighting's shadows stay inside it.
- Lighting looks for blockers with 48 samples, so objects thinner than Length ÷ 48 can be
  missed. Bend, Wave and Four point warp move pixels, not paths: a shape layer's outline is
  not re-drawn sharp after them.
- Four point warp's edges bow with one Curve each instead of two bézier handles.

## For developers

- Shader sources: `D:\Siceliot\extensions\src`. The shared ramp lives once in
  `ramp-params.wgsl`, `ramp-fns.wgsl`, `spread-params.wgsl` and `spread-fns.wgsl`;
  `sh build.sh` splices them into each effect, writes `extensions\siceliot\*.wgsl` and checks
  them. Never edit the output files.
- `tools\lfx-check`: reads an extension exactly as Lumit does (`build_passes`) and validates
  every pass with naga, the compiler wgpu uses. It lists the rows each effect gets.
- `tests\render`: `make-base.js`, `seed.ps1`, `render.js`, `chain.js`, `render2.js` and
  `render3.js` render the pictures on this page with `lumit-cli`. `showcase.sh` installs a test
  copy of the extension with a few defaults changed (named in `render3.js`), because scripts
  cannot set LFX rows.
