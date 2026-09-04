# Shape layers — the plan before the code

**Status: built, first cut.** The durable parts now live in
[../03-DATA-MODEL.md](../03-DATA-MODEL.md) §7.2, [../06-RENDER-PIPELINE.md](../06-RENDER-PIPELINE.md)
§1.2 and [../07-UI-SPEC.md](../07-UI-SPEC.md) §2.3.1; what is left here is the reasoning behind
the choices and the record of how the plan turned out. Two things went differently
from the plan and are worth knowing:

* **The renderer is CPU, and reuses two rasterisers rather than adding one.** A
  fill goes through `mask::rasterise` (the same coverage a mask is gated by); an
  outline goes through `paint::apply_strokes` (a brush run along the flattened
  path). The plan left CPU-versus-GPU open and said the GPU was the honest
  long-term answer — it still is, and it changes the rasteriser without changing
  anything stored.
* **The trap was real.** `LayerBoundsCache` did follow the revision, as the plan
  hoped, but both sides had to agree on *how* the box is measured: by control
  points, not by the curve. That is now written down in both places and tested on
  both sides.

**The modifiers arrived as fields, not as a tree.** After Effects keeps
Trim Paths, the Repeater and the rest as entries in a nested group, and their
*position* in that group is what decides what they act on. Lumit's list is flat
and has no positions to read, so each modifier is a property of the item it
modifies and the order they apply in is fixed and written down
([../03-DATA-MODEL.md](../03-DATA-MODEL.md) §7.2.1). The cost is that you cannot
trim two items as one; the gain is that a modifier is a `Property` beside every
other `Property`, so it keys, undoes, previews and crosses the bridge with no new
machinery at all. Every modifier is left out of the file until it is used, so the
tree §9.2 still plans re-homes these fields rather than replacing them.

Not built: nested groups, wiggle paths, and joins and caps other than round.
Animated paths **are** built — the mask's own keyed path, item by item.
Dragging a shape's points on
the picture **is** built: the gesture that mask points already have serves shape
contents too, since both hold the same `BezierPath`. Mind the coordinates: a shape
item's vertices are in the *art's* space, and the layer's pixels start at the art's
bounding-box corner, so anything drawing or hit-testing a shape point subtracts that
corner (`LayerBox.shapePoint`), and anything writing points back leaves position to
follow the corner — `set_shape_contents` does it, as one `Op::Batch`.

## In plain terms

A **shape layer** is a layer whose content is vector art rather than pixels: you
draw a rectangle or a path, and the layer *is* that shape — filled, stroked, and
resolution-independent, so it stays crisp at any scale. After Effects makes one
whenever you drag a shape tool with nothing selected. Lumit could not, when this
plan was written: `LayerKind` had footage, solid, precomp, text, camera, sequence,
adjustment and null in it, and no shape. That was the gap this plan closed.

A mask (shipped) is a *path on another layer* that decides which pixels
show. A shape layer is a path that **is** the picture. The geometry is the same
`BezierPath`; everything else differs.

## What has to be built, in order

1. **The model** (`lumit-core`). `LayerKind::Shape { contents: Vec<ShapeItem> }`,
   where a `ShapeItem` is a path plus its paint — fill colour, stroke colour,
   stroke width — each animatable (`Property`) like every other value in the
   document. Start with one item per layer and a flat list; AE's nested groups
   and its shape *modifiers* (repeater, trim paths, wiggle) are later work and
   must not shape the first cut.
   The path itself is `mask::BezierPath`, unchanged — one path type in the
   document, drawn by two things.
2. **The renderer** (`lumit-render`, `lumit-gpu`). A shape layer has no source
   pixels, so `build.rs` needs a draw kind that rasterises a path at the size the
   frame is being rendered at: fill by non-zero winding, stroke as a widened
   path. The natural size (the wireframe reads it) is the path's bounding box.
   Decide early whether rasterising happens on the CPU into a texture (simple,
   costs an upload per change) or on the GPU by tessellating (fast, and the only
   honest answer for a shape being animated). The second is right; the first is
   a legitimate first commit if it keeps the seam identical.
3. **The bridge.** `add_shape_layer(contents)`, the contents in the read model
   beside `masks`, and edits through the same whole-list op the masks use.
4. **The frontend.** The shape tools' "nothing selected" branch stops posting its
   notice and makes one of these instead (`viewer_shape_layer.dart`,
   `_sayNoLayer`). The Timeline's twirl-down grows a Contents heading beside
   Masks and Effects. The layer-kind icon and the Project panel's kind column
   need the new kind.

## Decisions taken already, so they are not re-taken

- **The path type is shared with masks.** One `BezierPath` in the document, one
  set of maths, one bridge vertex type (`BridgeVertex`). A shape layer's
  path and a mask's path differ in what they *do*, not in what they are.
- **Layer space, as everything else.** A shape's coordinates are the layer's own,
  so the layer's transform moves the shape exactly as it moves a mask.
- **Nothing is dressed up as a shape layer until there is one.** Until the kind
  exists the tools say so. A solid with a mask would be a lie in the
  layer list.

## The modifiers, one at a time

**Trim paths.** The hard part was already written: a paint stroke's write-on
cuts a polyline by arc length, and that is exactly what a trim is. The
shape side flattens the bezier, cuts with the same function, and hands the piece
back. Three things were decided rather than derived:

* **The fill is cut too.** After Effects closes the surviving piece and fills
  that, so a half-trimmed circle is a filled half circle rather than a
  half-outlined whole one. A polyline is a `BezierPath` with every handle at
  zero, so the piece goes through the *same* fill rasteriser the whole path does.
* **The offset moves the seam, not the window.** For a closed path, sliding the
  trimmed piece round can put it astride the point the path starts at, which
  would be two pieces to draw. Re-starting the polyline `offset` per cent along
  instead makes it one contiguous piece again, and the ordinary trim then cuts
  it. An open path has no seam, so it gets the window shifted and clamped: slide
  it far enough and it runs off the end, which is the honest answer.
* **An untrimmed item never sees a polyline.** `trims_at` is the guard: with the
  trim at 0..100 and no offset the item is rasterised from its curve exactly as
  it always was, so the identity case is byte-for-byte what it was before there
  were modifiers.

The layer's **natural size is still the untrimmed box**, for the reason a paint
stroke's bounds give: a box that shrank as a write-on played would make the layer
breathe, and every cache keyed on its size would churn.

**Dashes.** The same cut again: the outline is already a polyline being handed
to the paint rasteriser, so dashing it is walking the pattern along its length
and handing over several runs instead of one. Two things worth knowing:

* **The ceiling is deliberate.** A dash pattern is a length, and a path can be a
  million units long, so the piece count is unbounded in principle. Past 4096
  pieces the outline is drawn **solid** rather than truncated — at that density
  it is a solid line to the eye, so the wrong answer nobody can see beats the
  wrong answer that stops half way along.
* **There is no "add dashes" gesture.** Writing Dash or Gap on an item that has
  no list makes the pair. The alternative was a menu item whose only job was to
  put two zeros in a list, and a row that reads zero until you type in it says
  the same thing with nothing to find.

**The repeater.** The third modifier is the first that puts art where the path
is not, and that is the whole of its difficulty. Three things fell out of it:

* **The layer's box had to learn a clock.** A trim only ever takes art away, so
  the box could stay the untrimmed one; a repeater *adds* art outside the path,
  so the box has to hold the copies, and a keyed repeater moves them every
  frame. `bounds` and `contents_bounds` take a time now, and the frontend
  measures a shape layer fresh rather than from the revision-keyed cache. The
  price is real: a
  repeater keyed to grow up or left slides the art, because a shape layer's
  position is pinned to the box's corner.
* **The transform is six numbers here, not a matrix type.** This is the only
  place in `lumit-core` that composes transforms in the art's own space; a
  dependency for six multiplications would be a dependency for six
  multiplications. The frontend carries the same six for the wireframe, and the
  two are tested to agree.
* **The ceiling is the rasteriser, not the format.** Every copy is a scanline
  pass over the whole layer, so a hundred copies is a hundred passes. A count
  past `MAX_COPIES` is *held* rather than refused — the number is a slider —
  and lifting the ceiling means teaching the mask rasteriser to work in one
  copy's own box, which is a change over there rather than here.

**Gradient fills.** The ramp is the Gradient effect's arithmetic, written again
rather than called: `cpu::gradient` fills a whole f32 raster and replaces what
was there, so reusing it would have meant an f32 buffer the size of the layer
*per drawn copy* to composite back through coverage. What is shared is the
*reading* — the linear projection, the radial distance, the single epsilon on
the squared axis length — and the doc comments point at each other so the two
cannot drift without somebody noticing. The colours are mixed in **linear** and
encoded after, through a 256-entry table built once per drawn copy: a
transcendental per pixel done honestly is the alternative, and the table is
finer than an 8-bit result can show.

The gradient's points are in the art's own coordinates and are placed through
the *copy's* transform, which is what makes a repeated copy carry its ramp
rather than sample the original's.

**Path morphing.** The one item on the "not built" list above that turned out to be already
written. A mask's shape already keys, and `path_at`, `lerp_paths` and `resample`
between them are the *whole* of morphing — the resampling rule included, which is the part that
looks like it needs an algorithm and does not: the sparser path is cut into as many pieces as the
denser one has by de Casteljau, so the path that comes back is geometrically the path that went
in. So the work was almost entirely *plumbing*: `path_keys` beside a shape item's `path`,
`mask::path_at` pulled out of `Mask` into a free function the two callers share, and every read of
`item.path` in the rasteriser and the bounds turned into `item.path_at(t)`.

Three things worth knowing:

* **The ceiling is the correspondence, and it is AE's.** Two paths run vertex for vertex after
  resampling, with no attempt to *match* features: morph a triangle into a square and the corner
  the flat side grows from is decided by which vertex each path starts at, not by which corners
  look alike. Rotating a path's start point is the user's lever, which is what After Effects
  gives too. A matcher would be a real algorithm and a real argument about what "alike" means;
  the honest simple thing is here and the ceiling is written down.
* **The playhead had to reach four more doors.** `set_shape_contents` grew an `at`, exactly as
  `set_mask` has one, because once a path is keyed the stored `path` is not what draws and a
  point drag written there moves nothing. The **preview** door needed it too: a drag previews
  through `renderFrameWithShapePreview`, which rebuilt items from the bridge type and so dropped
  their keys — a drag on a morphing shape would have shown nothing moving until the release,
  which is the very bug the `at` exists to prevent.
* **The graph draws nothing for a shape yet.** A mask's path row reaches the graph editor as a
  channel whose value is the counted-up interpolation parameter; a shape item's rows —
  path, trim, offset, the repeater's nine — reach it as nothing at all, because
  `graphChannelsFor` knows about transforms, effects, retime and masks and has never known about
  shape contents. That gap is older than morphing and is not closed here; the lane diamonds are,
  which is what a keyed row most needs.

**Boolean combines.** The first modifier that could not be written here. Every one
before it is arithmetic on a polyline this crate already walks — cut it by length, push it
sideways, place a copy of it — and a boolean is not: it needs the points where two outlines
cross, and then a decision about which of the fragments those cuts make are inside the answer.
The ladder was climbed and nothing in the tree served. `mask::rasterise` gives coverage, not
geometry, so combining coverages would leave a stroke with nothing honest to follow; the offset's
own round-join walk knows nothing about intersections; there was no `lyon`, no `kurbo` and no
`geo` in `Cargo.lock` to lean on. **`i_overlay` 8** was added for it — six small crates,
MIT/Apache, and integer-grid inside, which is the determinism docs/14 asks for rather than
something to be argued about afterwards.

Four things fell out of it:

* **The surface is one `u32` on the item, and it points backwards.** `BezierPath` is one ring
  with no subpaths, so there was nothing inside an item to combine; the honest unit is two items
  in the same layer. `combine` says how *this* item joins the one *before* it, which makes a run
  out of a flat list with no new op, no new read model and no tree — the same trick the
  modifiers used to arrive without groups. Folding left to right means three items read
  `(A ∪ B) − C`, in the order they are written.
* **A run wears the first item's paint.** It has to wear somebody's, and the first item is the
  shape you started with; the ones after it are cutters. The cost is that a member's own rows
  describe nothing, so the panel leaves them out — the rule for a dash on a fill-only shape,
  applied again.
* **Even-odd, not non-zero.** `i_overlay` will read the input either way. The rasteriser reads
  it one way, and matching it is what makes joining a shape to something leave the shape looking
  the same. After Effects would fill a self-crossing star's middle; Lumit's does not, before or
  after a combine, which is the consistency worth more than the parity.
* **The fill had to learn to count across contours.** A subtract is two rings, and which pixels
  are inside depends on both at once — `mask::rasterise_paths` walks them together, and
  `rasterise` is now a one-element call into it. Rasterising each ring and combining the
  coverages afterwards would double-count every overlap.

The box is **not** re-measured: every boolean of two shapes lies inside the union of the two, so
the box the members already give is correct and never too small. A subtract leaves the layer
larger than its picture, which is the same generosity bounding a curve by its control points
already allows, and it costs nothing where working the combine out a second time each frame
would.

**Offset paths.** Thirty lines, and the temptation was to make it three hundred.
The polyline is offset segment by segment, the corners that open are filled with
a round join, and the corners that close are joined straight — which is where the
self-intersections come from. The alternatives were a proper polygon-clipping
library (a dependency and a determinism question, for a case a slider walks back
out of) or a miter join with a limit (a second join style, when the crate draws
exactly one). Neither earned its keep. The **winding** is read from the shoelace
area so a positive amount grows a path written either way round; that is one loop
and it removes the only way the feature could be silently backwards.

## The trap to expect

The wireframe and hit-testing read a layer's *content size* to draw its
box. A shape layer's size is its path's bounding box, which **changes as the path
is edited** — unlike every kind built so far, whose size is fixed by its source.
`LayerBoundsCache` caches by document revision, so it will follow, but the cache
was written when "a layer's size" was a constant and its comments say so; they
want revisiting rather than trusting.
