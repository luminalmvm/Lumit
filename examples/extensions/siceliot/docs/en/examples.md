# Example projects

Two projects in `examples\`: [Flood](#flood) (1080 × 1920, 30 fps, 20 seconds), after a scene
shared on [Scenery](https://scenery.io/scenes), and [Sketch](#sketch) (1920 × 1080, 60 fps,
5 seconds), after a Processing sketch. Everything in them stays editable: no setting is baked into
pixels, and every moving part is a keyframe or a driver wired into a row, which you can retune,
rewire or switch off.

*Italiano: [../it/esempi.md](../it/esempi.md)*

## Opening them

1. Use the Lumit built from this repository (`D:\Siceliot\Lumit\…\Release\lumit.exe`), not the
   official one: Type fill, the drivers and the Label points rows they use are engine changes.
2. **File ▸ Open project…** `examples\flood.lum` (comp *Flood*) or `examples\sketch.lum` (comp
   *Copies*).
3. The drivers are in each layer's **Node graph** (`Window ▸ Node graph`, with the layer
   selected).

`examples\make-flood.ps1` builds Flood again from scratch, and `examples\render.js` renders a comp
with `lumit-cli` (`--arg comp=… --arg from=… --arg to=… --arg dir=…`).

## Flood

After *Submesh Boolean* by Tim Siegert (Cavalry, [Scenery](https://scenery.io/scenes),
CC BY-NC 4.0): rows of "Flood the Grid" fill a 1080 × 1920 frame, and shapes come and go through
them. Outside the shapes the words are black on white, inside white on black, and on both sides
they stretch to fill exactly the room they have. 30 fps, 20 seconds; `examples\make-flood.ps1`
builds it.

![Flood](../images/example-flood.png)

- **Shapes** (a precomp): white solids cut to a shape by **Falloff** (Output Alpha, Softness 0),
  each on for four seconds: a diamond that opens and closes (0–4 s), a circle that rises
  (4–8 s), a giant G that grows and turns (Label points on a single Grid point, 8–12 s), two
  columns that part (12–16 s) and a tall ellipse (16–20 s). Sizes and positions are eased
  keyframes.
- **Outside**: **Type fill** with the Shapes layer as **Region layer**, **Invert region** and
  **Cut words** on (the shapes cut the words, never repeat them), its look from the hidden Text layer *Look black* (Impact, black).
- **Shapes** in the main comp, filled black with Fill: the inside.
- **Inside**: a second Type fill on the same region, not inverted, Cut words on, look from *Look white*.
- **Paper**: a white solid.

Retype **Words**, change the Text layers' font, or add shapes to the precomp: the words find the
new room on their own. **Rows**, **Row gap**, **Word gap** and **Min width** set the density.

Non-commercial use only, as the original's licence asks.

## Sketch

After the Processing sketch "lettere su lettere": a giant letter filled with small characters,
crossed by rings that colour, enlarge, turn and change them; a character a ring leaves stays
changed.

![Sketch](../images/example-sketch.png)

**How it is built.** The steps below rebuild it in any comp (the example's is 1920 × 1080,
60 fps). `tests\copies\wire-sketch.ps1` does the same from a script, and with `GRADIENT=1` it uses
Flash colour *Gradient*.

1. Make two Text layers and hide both. **Mask** holds a big bold letter (Arial Bold, 720 px) and
   decides which points stay. **Look** holds any character (Arial, 12 px, white) and gives the
   characters their look.
2. On a **Points** layer, add **Grid** with 75 × 75 points, Spacing 12 px and Mix 0 (no discs).
   Set its Position x and Position y by the expressions `comp_width / 2` and `comp_height / 2`
   (right-click the value, **Set expression**), so the grid stays on the comp's centre at any size.
3. Under it, add **Pick points** with Pattern *Image*, Image layer Mask, Map placement Centre,
   From 1 and Mix 0, and wire Grid's Points into it. Only the points on the letter stay, its soft
   edge included.
4. Under that, add **Label points**, its Points wired from Pick points. Set Text layer to Look and
   Text to `{pick:A|B|C|…|&|*}`, with Anchor Centre, **Vertical anchor** *Top*, Offset x 0,
   Max labels 2000, Change above 80, Scramble rate 60 and Flash colour *By strength*.
5. In the Node graph, add a **Ring wave**: Origin *Away from points*, Clearance 9, **Add up** on,
   Interval 0.5 s, Speed 300, Thickness 60–120, Grow 20–50. Wire **Value** into Change and
   **Passes** into Text step.
6. For size, add a **Math** (Add) with A = 100 and Ring wave's **Grow** in B. Its Value goes into
   Label scale.
7. For turn, add a **Value noise** (Vary by *Position*, Noise scale 100, Frequency 0.2, Detail 4,
   Minimum −18, Maximum 18). Feed it and Ring wave's **Rotation** into a second **Math** (Add),
   whose Value goes into Label rotation.

Where a ring's middle passes (Change above 80), the character scrambles every frame, then settles
on a new one that stays. Around it the letters grow, turn and take colour, and once the ring has
gone they go back to white and still. The rings start outside the letter and sweep across it.

- **Another size.** For a bigger letter, raise Grid's Columns and Rows as well, so the grid covers
  it.
- **Another layer.** Copy the Points layer's effects in Effect Controls and paste them on another
  layer. The Ring wave, the Value noise, the two Maths and every wire come along. Mask and Look
  are layers, not effects, so keep them in the comp.

Every row is explained in [effects.md](effects.md#label-points).

## Effects used

| Effect | Flood | Sketch |
|---|---|---|
| Type fill | ✓ | |
| Falloff | ✓ | |
| Grid | ✓ | ✓ |
| Pick points | | ✓ |
| Label points | ✓ | ✓ (By strength, Text step, Grow, Label rotation, Vertical anchor) |
| Ring wave | | ✓ |
| Value noise | | ✓ |
