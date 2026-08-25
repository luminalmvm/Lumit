# Paint — how a stroke becomes pixels

**Status: built (K-227), first cut.** The model, the CPU rasteriser, the bridge, the tools and
the Timeline rows are in. What is not in is named at the end, and none of it changes what is
stored.

## In plain terms

You drag a brush over the picture. Lumit does **not** keep the pixels you made — it keeps the
drag: the path your pointer took, in the layer's own coordinates, plus the colour, the width,
how hard the edge is, how opaque the mark is, and which of the three tools made it. Every time
that frame is drawn, the stroke is stamped again at whatever size the frame is being rendered
at.

That is the whole design, and everything else follows from it. Painting at a quarter-resolution
preview and exporting at full size gives a *full-size* stroke rather than a blurry quarter-size
one. Changing a stroke's colour a week later is a number, not a repaint. Undo removes one
stroke, because a stroke is one thing rather than a smear of changed pixels.

## The shape of a stroke

`lumit_core::paint::PaintStroke` — id, name, `points` (layer space, in the order drawn),
`pressures`, `colour`, `width` (diameter), `hardness`, `shape`, `opacity`, `start`, `end`,
`mode`, `blend`, `clone_offset`.

A **polyline**, not a bezier. Masks and shape layers are the bezier things; nobody edits a
stroke vertex by vertex, and the points are samples of a gesture rather than a designed shape.
They are thinned before they are stored (`thinStroke` in `viewer_paint.dart`): samples closer
than two screen pixels to the last one kept are dropped, and the first and last always survive.
A slow drag can raise several hundred pointer events a second; a thousand-point path costs the
renderer for nothing anyone can see.

**`pressures` is a list beside the path** (K-583), one 0..1 per point: how hard the stylus was
pressed there. It is a parallel list rather than a third number inside each point because
**empty has to mean something** — empty is the constant 1.0 everybody painted with, so a
mouse-drawn stroke and every stroke drawn before there was a stylus to read write exactly the
bytes they wrote yesterday and stamp exactly the pixels they stamped. A list that stops short
is read the same way, point by point: a missing entry is 1.0, and so is a NaN, because this is
a file the engine did not necessarily write and a stroke that draws is a better answer than one
that vanishes. The frontend thins the pressures with the points, through the same
`thinStrokeIndices` — a second thinning that happened to agree would be a second thinning that
one day did not.

## Stamping (`apply_strokes`)

Beside `mask::apply_masks`, and called from the same place in `lumit-render`'s `build.rs`, for
the same reason: the layer's pixels are already in hand, in a buffer whose size is whatever
this frame is being rendered at.

1. **Scale.** The stroke is in layer coordinates; the buffer is `w × h` for a layer whose
   natural size is `natural_w × natural_h`. One scale factor per axis, and the *smaller* is
   used for the brush radius so a round brush stays round.
2. **Trim** (K-549). `PaintStroke::drawn_at(t)` cuts the path down to the piece between `start`
   and `end` per cent of its own **arc length** at this frame, and everything below works on
   that piece — the bounding box included, so a write-on reserves only what it has drawn. Per
   cent of *length*, not of the sample count: a gesture's samples bunch up wherever the hand
   slowed down, so counting them would make a write-on speed up and slow down with the drawing.
   An `end` at or below `start` is an empty path and the stroke is skipped — that is a
   write-on's first frame, not an error. A path with no length (a single dab) has nothing to
   cut, so it is drawn whole as soon as anything of it is asked for. Both are absent from the
   file at their defaults (0 and 100), so an untrimmed stroke writes the bytes it always wrote.
3. **Dabs.** Each segment of the trimmed polyline is walked at a quarter of the brush radius
   and a dab is stamped at each step, plus one at the far end so a stroke never falls short of
   where the pointer stopped. A one-point stroke is one dab.

   **Pressure is a scale on the radius, per dab** (K-583), and nothing else: `radius ×
   pressure`, never below half a pixel, with the hardness ramp worked out from *that* radius so
   a light dab softens in proportion rather than turning to fog. A pressure of 0 is no dab at
   all — the pen lifted. The dab spacing follows the pressure too, measured from the lighter of
   the segment's two ends, because a light dab is a smaller dab and spacing taken from the full
   radius would leave a dotted line wherever the hand went light; it is floored at a tenth so a
   pressure sliding towards nothing cannot ask for an unbounded number of invisible dabs. At
   pressure 1 every one of these numbers is the number the rasteriser has always used, to the
   bit, which is the point.
   The **trim carries the pressures with it**: `trimmed_with` is the one walk, lerping the
   pressure at the two segments the ends cut into, so a write-on of a pressed stroke thins
   where the whole stroke thins and no second copy of the arc-length arithmetic exists.
4. **Coverage.** Each dab writes a soft falloff into a 0..255 coverage buffer, taking the
   **greatest** value at each pixel rather than adding. Greatest, not sum: the dabs overlap
   heavily by design, and adding them would make the middle of a slow stroke opaque and its ends
   thin. The stroke's own opacity is applied once, at composite time.
   The falloff is `1` inside `radius − feather`, `0` outside `radius`, linear between, where
   `feather = radius × (1 − hardness)` and never less than half a pixel — a perfectly hard edge
   would stair-step otherwise.

   **The shape is one substitution** (K-548). `Dab::distance` decides what "how far from the
   centre" means — the straight line for `BrushShape::Round`, the greater of the two axes for
   `Square` — and *every* number downstream is written in terms of it: the radius, the hardness
   ramp above, the bounding box, the dab spacing. So a square softens exactly as a round does,
   outwards from a flat-sided core, and there is no second case anywhere. Two shapes is the
   whole feature: no bitmap tip, no angle, no roundness, and no `if` waiting for them. `shape`
   is left out of the file while it is Round, so a project nobody has re-shaped writes the bytes
   it always wrote and keeps the frames the cache has banked.
5. **Composite,** by mode:
   * **Paint** — source-over with the stroke's colour, alpha `coverage × opacity × colour α`.
   * **Erase** — `dst.a ×= 1 − coverage × opacity`. Colour untouched, which is what makes an
     erase reversible by lowering its opacity later.
   * **Clone** — sample the layer at `pixel + clone_offset × scale`, source-over. Off the layer
     copies nothing (wrapping reads as a bug).

**The blend is a colour, never a coverage** (K-550). `blend` is a
`lumit_core::model::BlendMode` — the layer list, the same words in the same order — and it
decides what colour the mark lays down, not how much of the pixel it lays it on. Per pixel,
when the mode is anything but Normal: decode the destination and the mark's colour to linear
light, run `fx::cpu::blend_pixel` (the one shared kernel, the same one an effect's Blend row
uses, so the domains are the compositor's — docs/06 §blend domains), encode the answer back, and
hand *that* colour to the same source-over. Normal takes an early exit, so an unblended stroke
is byte for byte what it was, and `blend` is absent from the file until it is chosen.

Erase ignores it: an erase has no colour, and a mode there would be a second way of saying
nothing. Clone honours it, blending the *sampled* colour.

**The clone trap.** Clone reads a copy of the raster taken *before any stroke in the pass was
stamped*. Sampling the live buffer means a clone picks up paint laid down earlier in the same
pass — including its own output a few dabs back — and smears it across the picture. The copy is
only taken when a clone is actually present; most strokes are not clones and a copy of a 4K
layer is not cheap.

## Where it sits in the render

Paint, then masks, then effects (docs/06 §1.2). A mask gates the *painted* picture, and effects
see it. Two knock-on effects worth knowing:

* A flat solid is normally rasterised as an 8×8 tile and stretched. A layer with paint on it
  needs real pixels to mark, so it is rasterised at its true size — the same exception masks
  already forced.
* Paint on a **collapsed Precomp** layer forces the nested intermediate, exactly as a mask does:
  splicing a collapsed precomp never produces the layer's own raster, and there would be nothing
  to stamp into.

## Every kind of layer, and the one that needed help (K-547)

`pixels_for` builds a layer's raster and then stamps into it, whatever made it: a decoded
footage frame, a Sequence clip, a solid at its true size, a rasterised line of type, a shape
layer's art. One shared tail, so a stroke behaves the same on all of them. Two consequences
worth knowing: a shape layer's raster **is** its art's bounding box, so the layer's pixel
(0, 0) is that box's corner (K-308) and re-drawing the art far enough to move the box moves
the paint with it; and a text layer's raster is the line as typed, so an expression-driven
caption that grows re-lays its strokes against the new width.

A **Precomp** had no raster at all — a composition has no pixels until it is rendered, and it
is rendered on the graphics card — so `pixels_for` answers `None` for one and the strokes were
dropped. `DrawSource::Nested` now carries the layer's `paint`, and `Realiser::paint_over`
stamps it once the nested comp has been realised:

1. `ColourEngine::display` with `DisplayParams::NEUTRAL` — never the Viewer's exposure or tone
   map, so the picture a preview paints on is the picture an export paints on (K-031);
2. `readback8`, giving the same 8-bit sRGB bytes every other layer is painted in;
3. the **same** `apply_strokes`, with the nested comp's own width and height as the natural
   size, so a reduced-resolution preview scales the brush exactly as it does everywhere else;
4. `upload_srgb8` + `linearise` back into the working texture.

A read that fails returns the unpainted picture rather than panicking, and an empty stroke list
returns the texture untouched — a Precomp nobody has painted on pays nothing. The cost when
somebody has: one synchronous read-back per frame, and the nested picture quantised to eight
bits. Both are named in "Not built" below as the GPU stamping pass's job.

A Precomp used as a **track matte** or as an effect's layer input renders unpainted: those
build a `NestedInputDraw`, not a layer draw, and take the same v1 boundary the source's own
effects take (K-266).

## The seam

`SetLayerPaint` replaces the whole list and is exactly invertible — the same shape as
`SetLayerMasks`, so an add, a delete, a rename and a recolour are one kind of edit and each is
one undo step. The bridge carries `BridgeStroke` (with `BridgeStrokePoint`, named for the stroke
because `BridgePoint` is already an animatable effect parameter), clamps every number that would
render wrongly for ever after, and refuses a stroke with no points. Strokes ride the read model
(K-184) beside the masks so the Timeline lists them without asking per row per frame.

## The tools

`viewer_paint.dart`. The stroke in flight is drawn on the overlay at the brush's width — a
gesture that has not happened yet cannot go through the engine's preview path, which patches
*values* into a copy of the document rather than lists. On release the whole stroke is committed
once. `Escape` abandons; `Backspace` calls `delete_last_stroke`.

**The pressure arrives free.** `PointerEvent.pressure` is on the raw stream the `Listener`
already wrapped round the gesture detector, and a hit-test target is handed each event before
the gesture arena routes it to a recogniser — so the pressure read in `onPointerMove` is the
press that made the point `onPanUpdate` is about to add. It is normalised against the device's
own `pressureMin`/`pressureMax` (a tablet reporting 0..1024 is not taken at face value), and
anything that is not a stylus is a flat 1.0. The brush's **pressure controls size** toggle in
the tool options decides whether the captured presses are committed at all; off commits 1.0
everywhere, which the bridge stores as no pressures, which is the stroke a mouse would make.

The clone stamp needs `Alt`-click first, which sets the source in *layer* coordinates so it
stays put on the picture while the view is panned and zoomed. The offset committed with a stroke
is `source − first point`, so the whole stroke keeps the relationship the first dab set.

## Test plan (implemented)

* **Core** (`paint.rs`): a dab marks where it was put and nowhere else; a stroke joins its
  points up with no gaps; soft and hard brushes differ halfway out; a square brush fills the
  corners a round one leaves, softens at its flat edge by the same ramp, and is absent from the
  file until it is chosen; Start and End trim by length at either end, an End at or below Start
  draws nothing, the trim measures length rather than samples, a single dab has nothing to trim,
  and an untrimmed stroke is absent from the file; a blend combines the mark with what is under
  it, changes the colour and not the coverage, does nothing on an erase, and is absent from the
  file at Normal; opacity scales the mark;
  the same stroke at half resolution is half the size in pixels; erase takes alpha and leaves
  colour; clone copies from its offset; clone reads the layer as it was; empty, zero-width and
  zero-opacity strokes do nothing; a stroke off the layer is skipped; bounds include the brush
  width; painting twice gives identical bytes (determinism, docs/14);
  pressure scales the width of a dab by the number given (a numeric row: 40px, half, a
  quarter, nothing), a stroke thickens where it was pressed harder without breaking where it
  was light, a trim lerps the pressures at its cut and leaves an unpressed stroke's empty, an
  unpressed stroke is absent from the file, a full press everywhere paints exactly what no
  pressure does, and a short or absurd pressure list is read calmly.
* **Render** (`build_tests.rs`): a stroke reaches the layer's pixels, and a painted solid is
  rasterised at its real size rather than as a tile.
* **Bridge** (`api/tests.rs`): a point's pressure round trips, is clamped to 0..1, and a stroke
  nobody pressed reads back as a full press throughout;
  add/read/undo/redo in one step; strokes ride the read model;
  edit and delete by id with a calm error for a stale id; the last stroke can be taken back; an
  empty stroke is refused; absurd numbers are clamped; all three modes and a clone offset round
  trip; both brush shapes round trip and an unstated one reads back Round; a stroke's Start and
  End round trip, keep their keys, and are clamped to 0..100 key by key; a blend round trips by
  index and an index nobody has heard of reads back as Normal.
* **Frontend**: `thinStroke` and `paintModeFor` as pure tests; a brush drag paints one stroke
  that undoes in one step; the brush commits the shape chosen in the tool options; the eraser
  and clone stamp commit their own modes and the clone refuses without a source; painting with
  nothing selected says what to do; the Timeline grows its Paint heading and its rows write
  through, and a stroke's Start and End rows write through under it; a stroke's row picks a
  blend from the layer list; `thinStrokeIndices` agrees with `thinStroke` so a pressure is
  dropped with its own point, a mouse and a touch always press fully while a stylus is read
  against its own range, and a stylus drag commits the pressure it was drawn with — a full
  press everywhere once the brush's toggle is off.

## Not built

**Tilt**, and here is why. Honouring it needs a brush tip that can *turn*, and there is
deliberately no angle anywhere in `BrushShape`: K-548 refused the brush-tip system that would
carry one. Angling a round dab is a no-op dressed as a feature, and angling the square one is
that refused system arriving by the back door. So tilt is owed on a **shaped brush with an
angle**, not on this rasteriser, and the stylus's tilt fields are read by nothing today.

Also: spacing and scatter; painting in Layer view rather than on the composite; a stroke's Start/End **curve in the graph editor** (the lane
draws its diamonds and they drag, but `graphChannels` walks transform, effect and mask paths
only in v1); and a GPU stamping path. The last one is the only one that would change any code
here rather than adding to it — and it changes the *rasteriser*, not the stored stroke, which is why the
storage was decided first. It is also what would retire the 8-bit read-back a painted Precomp
pays (K-547).
