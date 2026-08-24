# Shape layers — the plan before the code

**Status: built (K-237), first cut.** The durable parts now live in
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

**The modifiers arrived as fields, not as a tree (K-451).** After Effects keeps
Trim Paths, the Repeater and the rest as entries in a nested group, and their
*position* in that group is what decides what they act on. Lumit's list is flat
and has no positions to read, so each modifier is a property of the item it
modifies and the order they apply in is fixed and written down
([../03-DATA-MODEL.md](../03-DATA-MODEL.md) §7.2.1). The cost is that you cannot
trim two items as one; the gain is that a modifier is a `Property` beside every
other `Property`, so it keys, undoes, previews and crosses the bridge with no new
machinery at all. Every modifier is left out of the file until it is used, so the
tree §9.2 still plans re-homes these fields rather than replacing them.

Not built: nested groups, wiggle paths, joins and caps other than round, and
animated paths. Dragging a shape's points on
the picture **is** built (K-307): the gesture K-224 gave mask points serves shape
contents too, since both hold the same `BezierPath`. Mind the coordinates: a shape
item's vertices are in the *art's* space, and the layer's pixels start at the art's
bounding-box corner, so anything drawing or hit-testing a shape point subtracts that
corner (`LayerBox.shapePoint`), and anything writing points back leaves position to
follow the corner — `set_shape_contents` does it, as one `Op::Batch` (K-308).

## In plain terms

A **shape layer** is a layer whose content is vector art rather than pixels: you
draw a rectangle or a path, and the layer *is* that shape — filled, stroked, and
resolution-independent, so it stays crisp at any scale. After Effects makes one
whenever you drag a shape tool with nothing selected. Lumit could not, when this
plan was written: `LayerKind` had footage, solid, precomp, text, camera, sequence,
adjustment and null in it, and no shape. That was the gap this plan closed.

A mask (K-222, shipped) is a *path on another layer* that decides which pixels
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
   path. The natural size (K-217's wireframe reads it) is the path's bounding box.
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
  set of maths, one bridge vertex type (`BridgeVertex`, K-222). A shape layer's
  path and a mask's path differ in what they *do*, not in what they are.
- **Layer space, as everything else.** A shape's coordinates are the layer's own,
  so the layer's transform moves the shape exactly as it moves a mask.
- **Nothing is dressed up as a shape layer until there is one.** Until the kind
  exists the tools say so (K-222). A solid with a mask would be a lie in the
  layer list.

## The modifiers, one at a time

**Trim paths.** The hard part was already written: a paint stroke's write-on
(K-449) cuts a polyline by arc length, and that is exactly what a trim is. The
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

## The trap to expect

The wireframe and hit-testing (K-217) read a layer's *content size* to draw its
box. A shape layer's size is its path's bounding box, which **changes as the path
is edited** — unlike every kind built so far, whose size is fixed by its source.
`LayerBoundsCache` caches by document revision, so it will follow, but the cache
was written when "a layer's size" was a constant and its comments say so; they
want revisiting rather than trusting.
