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
`colour`, `width` (diameter), `hardness`, `opacity`, `mode`, `clone_offset`.

A **polyline**, not a bezier. Masks and shape layers are the bezier things; nobody edits a
stroke vertex by vertex, and the points are samples of a gesture rather than a designed shape.
They are thinned before they are stored (`thinStroke` in `viewer_paint.dart`): samples closer
than two screen pixels to the last one kept are dropped, and the first and last always survive.
A slow drag can raise several hundred pointer events a second; a thousand-point path costs the
renderer for nothing anyone can see.

## Stamping (`apply_strokes`)

Beside `mask::apply_masks`, and called from the same place in `lumit-render`'s `build.rs`, for
the same reason: the layer's pixels are already in hand, in a buffer whose size is whatever
this frame is being rendered at.

1. **Scale.** The stroke is in layer coordinates; the buffer is `w × h` for a layer whose
   natural size is `natural_w × natural_h`. One scale factor per axis, and the *smaller* is
   used for the brush radius so a round brush stays round.
2. **Dabs.** Each segment of the polyline is walked at a quarter of the brush radius and a dab
   is stamped at each step, plus one at the far end so a stroke never falls short of where the
   pointer stopped. A one-point stroke is one dab.
3. **Coverage.** Each dab writes a soft round falloff into a 0..255 coverage buffer, taking the
   **greatest** value at each pixel rather than adding. Greatest, not sum: the dabs overlap
   heavily by design, and adding them would make the middle of a slow stroke opaque and its ends
   thin. The stroke's own opacity is applied once, at composite time.
   The falloff is `1` inside `radius − feather`, `0` outside `radius`, linear between, where
   `feather = radius × (1 − hardness)` and never less than half a pixel — a perfectly hard edge
   would stair-step otherwise.
4. **Composite,** by mode:
   * **Paint** — source-over with the stroke's colour, alpha `coverage × opacity × colour α`.
   * **Erase** — `dst.a ×= 1 − coverage × opacity`. Colour untouched, which is what makes an
     erase reversible by lowering its opacity later.
   * **Clone** — sample the layer at `pixel + clone_offset × scale`, source-over. Off the layer
     copies nothing (wrapping reads as a bug).

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

## Every kind of layer, and the one that needed help (K-447)

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

The clone stamp needs `Alt`-click first, which sets the source in *layer* coordinates so it
stays put on the picture while the view is panned and zoomed. The offset committed with a stroke
is `source − first point`, so the whole stroke keeps the relationship the first dab set.

## Test plan (implemented)

* **Core** (`paint.rs`): a dab marks where it was put and nowhere else; a stroke joins its
  points up with no gaps; soft and hard brushes differ halfway out; opacity scales the mark;
  the same stroke at half resolution is half the size in pixels; erase takes alpha and leaves
  colour; clone copies from its offset; clone reads the layer as it was; empty, zero-width and
  zero-opacity strokes do nothing; a stroke off the layer is skipped; bounds include the brush
  width; painting twice gives identical bytes (determinism, docs/14).
* **Render** (`build_tests.rs`): a stroke reaches the layer's pixels, and a painted solid is
  rasterised at its real size rather than as a tile.
* **Bridge** (`api/tests.rs`): add/read/undo/redo in one step; strokes ride the read model;
  edit and delete by id with a calm error for a stale id; the last stroke can be taken back; an
  empty stroke is refused; absurd numbers are clamped; all three modes and a clone offset round
  trip.
* **Frontend**: `thinStroke` and `paintModeFor` as pure tests; a brush drag paints one stroke
  that undoes in one step; the eraser and clone stamp commit their own modes and the clone
  refuses without a source; painting with nothing selected says what to do; the Timeline grows
  its Paint heading and its rows write through.

## Not built

Pressure and tilt; brush shapes other than round; spacing and scatter; write-on (a stroke's
own start and end times, which is what makes paint animate in After Effects); per-stroke
blending modes; painting in Layer view rather than on the composite; and a GPU stamping path. The last one is the only one that would change any code here rather
than adding to it — and it changes the *rasteriser*, not the stored stroke, which is why the
storage was decided first. It is also what would retire the 8-bit read-back a painted Precomp
pays (K-447).
