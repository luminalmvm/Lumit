# Timeline interaction — implementation note

**Decision:** K-499 (the Keys rows carry the Layers editing anatomy, and the layer number
returns), K-500 (the timeline selection model: a marquee from any ground, additive Shift,
a property's name selects its keys). **Related:** K-441 (the Timeline's resting shape),
K-442 (Graph mode), K-455 (Keys returns), K-457 (a key's shape says its interpolation),
K-458 (the drawing is authoritative; the block tools), K-459 (one key size in both modes),
K-439 (the accent's jobs split), K-196/K-203 (property selection and letting go), K-190/
K-292 (snapping), K-208 (both halves move). This note is the *how* for **timeline
interaction quality**: the binding spec for every gesture the Timeline's three modes take,
the audited gap list against the approved drawings and the Caddis study, and the ordered
work packages. The drawings (`Caddis study/mockups/Keys.dc.html`, `GraphMode.dc.html`,
`Main.dc.html` and their style manifests) govern geometry per K-450/K-458; this note
governs behaviour.

## In plain terms

The owner's ruling that commissioned this step: *interaction is everything to the user and
we want it slick*. Caddis — the editor the redesign studied — is not slicker because it
has more features; it is slicker because every gesture lands: the thing under the cursor
always answers, feedback appears only under the hand and vanishes after, a drag can always
be escaped, and the cursor itself says what a press would do. Lumit's timeline already
*has* most of the machinery — the marquee, the block box with stretch handles, the Ease
popover, the snapping — but the audit below found that much of it cannot be reached by the
gestures a person actually makes: the box-select is dead across most of the resting
timeline, keyframes do not travel with the block being stretched (a one-character string
bug), the graph's tangent handles sit still while their key is dragged, and a keyframe's
mark draws a seam down its own middle. This note names each failure with its file and
line, states every behaviour as a sentence a test can check, and cuts the fixes into
packages one agent each.

---

## 1. Principles (from the study, binding here)

These are the study's distilled interaction rules (`Caddis study/notes-editor-ux.md` §3,
`notes-visual-qs.md` §11–12, PLAN.md §1.5), restated as Lumit law for this panel:

- **P1 — Feedback is transient and local.** Anything a gesture summons (snap line, badge,
  hint, ghost) appears under the cursor while the gesture runs and leaves no trace after.
  Nothing changes the resting state.
- **P2 — The cursor is the affordance.** Every distinct grab offers a distinct pointer
  *before* the button goes down: resize arrows on trims and handles, a move cursor on a
  bar's body, the scissors while the razor is armed. A surface that will do nothing shows
  the plain arrow.
- **P3 — A drag is one undo step, previews live, and can be abandoned.** Every drag stages
  in Dart, previews through the engine's patched clone where a picture changes, commits
  once on release — and **Escape while the button is down reverts it entirely**, writing
  nothing. (The Escape rule is the study's; today no timeline drag answers it.)
- **P4 — What is selected is one colour, and it is not the accent.** Selected keys, the
  block box, its handles and badge draw in `text_primary`; `animated` says "this is
  keyed"; the accent keeps its closed list (playhead, one filled button, active tab tick —
  K-439, docs/15 §3.1). Any selection mark in `accent` is a defect.
- **P5 — Everything visible is reachable.** If a key, band or handle is drawn where the
  user works, the obvious gesture on it must do the obvious thing — and the ground between
  things must always admit the marquee. A drawn mark that swallows gestures it does not
  use (an opaque tap-only band) is a defect.

---

## 2. The selection model (K-500)

One model for Layers mode lanes, Keys mode and the graph. Each sentence is testable.

### 2.1 Keys

- Clicking a key selects exactly that key and deselects the rest.
- `Shift`-click and `Ctrl`-click **toggle** the key in and out of the selection without
  touching the others. (Both modifiers additive, as shipped; no separate range gesture on
  keys — range selection is the marquee's job.)
- Selecting keys selects their **properties** too (every distinct row the selection
  touches), so the outline and the graph show what is in hand — as shipped
  (`onKeysSelected`, timeline_panel_frb.dart ~3361).
- **A marquee can start on any ground**: empty lane space, a shut layer's row beside or
  behind its bar, a Keys-mode layer band, and the area below the last layer. Only a
  control that uses the drag itself (a bar's own strip, a key's grab target, a block
  handle) may take the gesture first. Plain marquee **replaces**; `Shift`/`Ctrl` held when
  the drag starts **adds** to the standing selection. Held *when the drag starts*, and the
  shared `MarqueeSelect` is where that is read — at `onPanDown`, reported to the owner
  alongside the box: the graph asked `HardwareKeyboard` at the release instead, so letting
  go of Shift half way through a box turned an adding drag into a replacing one. One
  widget, one answer, both panes.
- A plain click on any ground deselects everything (K-203, shipped).
- **Clicking a property's name selects the property and all of its keys** — the name is
  the row's "select all" (After Effects' own gesture). `Ctrl`-click toggles the property's
  keys in and out of the standing key selection; `Shift`-click extends the visible run of
  rows and takes their keys with it. Clicking a **layer's** row selects the layer, as
  today, and does not touch the key selection. The **stopwatch stays what it is** — the
  animate toggle, never a selector.
- Two or more selected keys **are the block**: the box, its two stretch handles and the
  `n keys · n f` badge appear whether the selection came from the marquee, from clicks, or
  from a property name — the overlay draws from the selection alone (shipped:
  `_KeyBlockOverlay` keys off `_selectedKeyPlaces`).
- **Right-clicking a lane key** opens the same menu the graph key has — Linear / Ease /
  Hold / Delete key — plus *Ease…* (the popover on the current selection). A right-click
  on an unselected key selects it first; on a selected one it acts on the whole selection.
  **A block's outermost keys answer through their handle.** A block's two stretch handles
  stand exactly over the first and last key of the selection, and a handle is opaque and
  drag-only, so those two keys once took neither a click nor a right-click — the carve-out
  above read literally, at the cost of the ordinary gestures P5 promises. Closed in TI-2:
  the handle carries a tap and a secondary tap of its own and **passes both to the key it
  is standing over** (`_keyUnderHandle`, the block overlay), so a click selects that key
  alone and a right-click opens its menu. A handle that answers a tap has a tap recogniser
  beside its drag one, so the drag is taken **from the down** rather than from the slop
  (`dragStartBehavior: DragStartBehavior.down`) — otherwise the mark would sit a pointer's
  width behind the cursor for the rest of the gesture.
- `Ctrl`+click on empty lane space of a **keyed** row plants a key at that time on that
  property (docs/07 §4.3's sentence; the graph shipped it first). The new key takes the
  value its own curve already reads there, so planting one moves nothing — it is a place to
  grab. A row with nothing keyed is left alone: turning a still property into an animated
  one is the stopwatch's job. Both lanes and graph write through one helper,
  `plantKeyOnChannels` (graph_editor_frb.dart), so a two-axis row's key lands on both axes
  in one op — one lane diamond is one key, and one undo step.

### 2.2 Deselection and scope

- Selection lets go exactly as K-203 says: closing a fold drops what was inside it,
  selecting a layer clears the property selection, empty-ground click clears all.
- The lane, Keys and graph key selections are one selection wherever the id space allows
  (a lane key fans out to its axes' channels — shipped, `_actionKeySelection`).

### 2.3 Keys mode's ground

- **Keys mode opens with every listed layer twirled open.** The dope sheet's point is the
  flattened property rows; a sheet of shut bands shows nothing and takes no marquee. The
  Keys twirl set starts as "all open" (its own default, still toggleable per layer) rather
  than inheriting Layers mode's shut-by-default set. Held as the **inverse** set —
  `_keysShut`, the layers the user has closed — so "all open" is the resting state without
  a pass over the layers to seed it, and so a layer opened here is not opened in Layers
  mode, where shut-by-default is the right answer. Everything that opens or shuts a layer
  (the twirl, `U`, the reveal keys, `L`) goes through one pair of helpers — `_setOpen` and
  `_shutLayerDeep` — which write whichever set the mode showing owns.
- A Keys-mode **layer band selects on tap but never swallows a drag**: drags pass through
  to the marquee. (`_KeysLayerLane` was an opaque `GestureDetector` and ate both. It is now
  translucent over an `IgnorePointer`ed child — a coloured child hit-tests for itself, so
  ignoring it is as much of the fix as the behaviour flag is. Both recognisers then stand
  in the arena, the band's tap in front of the marquee's pan, and a drag past the slop
  takes the box.)

---

## 3. The outline

### 3.1 Layers mode

As built and ruled (K-461/K-462/K-463); no interaction change from this note beyond §2's
property-name rule. Hover on a row washes the row one step (`surface_2`), P1-transient.

### 3.2 Keys mode rows (K-499 — the owner's finding *a*)

The shipped `_KeysOutline` (timeline_panel_frb.dart:6331) draws name-only property rows
with a read-only value, and its layer row (`_KeysLayerRow`, :6436) dropped the layer
number. The owner's ruling — Keys rows **match the Layers fold rows** — supersedes the
drawing's read-only labels (K-458's own carve-out: the owner personally says otherwise)
and §12A.1a's bullet. The exact anatomy, recorded:

- **Layer row**: twirl · **layer number** (muted mono, the same column K-461 gives Layers
  mode) · label dot · name · `n properties` count hard right. Raised on `surface_2`,
  selection fill when picked — as built otherwise.
- **Property row**: **stopwatch** (square under Sharp, the shared
  `keyframe_controls_frb.dart` control) · **◄ ◆ ► navigator** (drawn once animating) ·
  the name as the drawing writes it — `Group · Name`, group muted, a middle dot between —
  · the **value in an editable well**, scrub-drag and click-to-type, the same
  `transform_rows_frb.dart` / `effect_param_row_frb.dart` machinery the Layers fold-out
  and Effect controls share, so a parameter behaves the same wherever it is shown
  (docs/07 §4.3's one-implementation rule). The resting value text keeps `animated` — on
  this sheet every listed row is keyed.
- **Built in TI-4 by making a Keys row *be* a Layers fold row**, not by drawing a second
  one: `_KeysOutline` lists `_FoldRow` itself, which is what makes "the rows match" a
  fact rather than a resemblance to maintain. Three things the note asked for bent to
  that, and the fold row won each time, because a second answer is the thing K-499 was
  ruling against:
  - the **navigator has no reserved slot** — it draws where the fold-out's loose gutter
    puts it, once the row is animating. The reserved slot is the Effect controls' fixed
    columns (K-443); the fold-out is not measured in them, and on this sheet the slot is
    never empty anyway (Animated, the default, lists only keyed rows).
  - the **unit rides where its own row puts it**: inside the well as a suffix on a
    transform axis (`100%`), outside it as a rider on an effect parameter. That is one
    parameter behaving the same wherever it is shown, which is the rule that matters.
  - `keysPropertyIndent` (the drawing's 30) now measures to the **stopwatch**, the first
    thing on the row, as a Layers fold row's indent measures to its own.
  Two rows gained the name gesture they never had — a mask's value row and a Flow row —
  so §2.1's property-name rule is true of every row the sheet can list, and the group
  prefix is one shared `flatGroupPrefix` inside each row's own label.
- The property-name click follows §2.1 (selects the property **and its keys**); the value
  well and stopwatch pick the row without taking its keys — K-334's press-selects meeting
  K-196's rule that a control which edits does not re-aim the *key* selection.
- The rows stay flat (no group headings) and the filters row stays as built.

### 3.3 Graph mode

Built in TI-6. Per K-442, §12A.2 and `GraphMode.dc.html` the Graph outline is:

- The **filtered animated list**: Show — Animated (default) / All, no Selected; one row
  per animated property, carrying an **include-in-graph tick** and a **swatch in the
  curve's colour**, the name, and the value at the playhead — `animated` when the row is
  shown in the graph, muted when unticked. It is the same flattened sheet Keys mode
  draws (`keysLayerRows`), under the graph's own Show filter, so a property has one row
  model wherever it is listed; the twirl set is Keys mode's own `_keysShut` for the same
  reason (`_flatSheet`), and a layer opened on one sheet is open on the other.
- **The tick is the selection.** A ticked property is a property in `_selectedProperties`
  — the set the Layers outline picks with and the set `graphChannels` reads — so there is
  one answer anywhere in the panel to "which curves are up". Ticking is `Ctrl`-clicking a
  name without the modifier; clicking the **name** selects that property alone and takes
  its keys with it (§2.1). A tick never carries the row's keys: it says what to look at,
  and selecting keyframes is the name's job (K-196).
- **Two departures from the drawing, deliberate.** The drawing splits a two-axis property
  into `Position · X` and `Position · Y` rows, each separately ticked; the row here is the
  **property**, because the tick's granularity is the property — a per-axis tick would
  need a second membership set beside the selection, and one selection is worth more than
  one row per curve. A multi-axis row draws **one swatch per axis** instead, exactly as
  the Layers fold row already names both of its curves. And an **unticked** row's swatch
  is muted rather than coloured: an unticked property has no curve, so it has no colour
  to promise.
- A **Normalise** checkbox at the far right of the filter row. **Its scaling, decided
  here** (the open question closed): each shown curve is fitted to **its own min–max**,
  not to a shared symmetric range — the point is that a rotation in degrees and an
  opacity in per cent both fill the height. It is implemented as a **range per channel**
  (`GraphEditorFrbState.rangeOf`) rather than as a scaling of values, so every coordinate
  in the pane stays in the property's own units and a pointer's y still comes back as a
  real value: a drag under Normalise writes exactly what it would write without it. The
  shared range — the grid and the gutter's numbers — becomes 0–100 read as per cent,
  because with unlike units on one pane there is no single value axis left to label. Each
  channel's own range is fitted from the **document's** keys, not the shown ones, so a
  normalised curve does not rescale under its own drag. A view setting, never data.
- A **Key readout row** pinned at the outline's foot while exactly one key is selected:
  `KEY f<frame> <value><unit>` then `In [well] % Out [well] %` — the influence wells
  editable, committing through the same tangent write the handles use (a side becomes a
  bezier at its current speed and the influence asked for). Two or more selected keys are
  a block, whose badge is the readout, and the row draws nothing at all rather than an
  empty strip.
- A setting restores an outline identical to Layers mode (K-442): Settings ▸ Interface ▸
  Panels ▸ *Graph mode keeps the Layers outline*, off by default. It is also the only way
  to reach a **value well** while the graph is up — the graph's own outline reads values
  rather than editing them — which is what the K-333/K-334/K-336 regression tests use.

---

## 4. The lanes

### 4.1 Bars

Shipped behaviour stands (move/trim with source bounds, ghost extent, corner marks,
K-208's both-halves slide, selection on the raw down). Additions:

- The bar's **body** shows the move cursor (`grab`, `grabbing` while held); its end strips
  keep `resizeLeftRight` (shipped, :10025). A locked bar shows the plain arrow —
  `forbidden` only *during* a refused drop, per P1.
- A bar hovered (not selected) firms its leading edge to full strength; nothing else
  changes (P1). Selection keeps brightening the fill (§12A.1).
- **Bar drags snap** (§4.5's still-to-build): both ends are sources against the shared
  target list, nearest capture wins, the caught target draws the same capture line a lane
  key drag draws, `Ctrl` suspends. The arithmetic is `timeline_snap.dart`; this is wiring.
- **Escape mid-drag reverts** the bar to where it started and writes nothing (P3).

### 4.2 Lane keys

- The mark is drawn per K-457/K-459 (11px, split halves) by the **seamless painter of
  §5**; its grab is the shipped 12px slot at full lane height.
- Cursor over a key: `resizeLeftRight` (shipped) — it is a horizontal-only drag.
- A key drag selects on the down, moves in time, snaps with target indication, commits
  once (all shipped); **Escape reverts** (TI-2, P3 — the shared `DragEscape`).
- A hovered key brightens to `text_primary` at half strength — the pre-selection hint —
  and its time/value appear nowhere until the drag starts (P1).
- **While a key drag or block stretch runs, a badge under the pointer reads
  `f<frame> · <value>`** (frame only for a multi-key stretch: `f<first>–f<last>`), in the
  block badge's own 8px mono on `surface_4`. This is the drawing's value-hint pill and the
  study's "live readout"; it vanishes on release. Both halves shipped: the key drag's
  `f<frame> · <value>` and, from TI-2, the stretch's `f<first>–f<last>` beside the handle
  in hand. The value is the one the lane's **own
  keys** carry — a multi-axis row reads its lead axis, exactly as its diamonds do
  (`laneKeysOf`) — rather than the row's sampled reading: sampling crosses the bridge, and
  a drag would cross it on every pointer move (K-184). The pill flips to the key's other
  side where the axis has run out, so the readout is never clipped by the edge.
- A shut layer's **summary diamonds** stay a statement, not a target (K-441) — but the row
  they sit on admits the marquee everywhere the bar itself is not (§2.1).

### 4.3 The block tools (K-458)

Shipped: the box, the 3×6 end marks in 11px targets, the badge opening the Ease popover,
whole-frame stretch, one undo step, Reverse / Copy / Paste-at-playhead in the Keys strip.
Corrections and additions:

- **The keys travel with the stretch** (TI-2). They did not: `_KeyLaneState._frameOf`
  compared the stretch's key set against the **escaped literal** `'\${widget.rowId}#\$i'`
  — backslashed dollars, so the string was never interpolated, the `contains` never
  matched, and every diamond sat still until release while the box moved. The fix was the
  two backslashes; the regression test drags a handle and asserts a covered key's drawn
  frame moved before release (`timeline_block_test.dart`).
- The stretch handle **snaps its dragged end to the shared targets** (TI-2), not only to
  whole frames: the same capture line, `Ctrl` suspends. The pointer's own answer is kept
  unsnapped and the snap taken from it on every move, or a caught end could not be pulled
  off the target again; the dragged end alone is exempt from the whole-frame rounding,
  since a target (another row's key) need not sit on a frame, and every key between it and
  the anchor still rounds.
- The badge count/span updates live through the stretch (TI-2 — it measured the resting
  selection until then) and the whole gesture reverts on Escape (TI-2).
- The box, marks and badge follow the selection into **Layers mode** identically (shipped
  — one `_LayerArea` draws both modes).

### 4.4 The marquee

- Start conditions per §2.1 — any ground, including Keys-mode bands and bar rows off the
  bar. Additive with `Shift`/`Ctrl` held at drag start.
- The box draws a `text_primary` hairline with a faint wash — **not the accent** (P4).
  Landed in TI-2 (`widgets/marquee.dart`, `marqueeWashAlpha`); the graph's marquee is the
  same widget, so it restyled with it, and `chrome_primitives_test.dart` holds the pair.
- The catch walk steps by each layer's **real block height**: `_keysIn` and
  `_selectedKeyPlaces` (timeline_panel_frb.dart, in `_LayerArea`) advanced by `rowHeight`
  per row and ignored `sequenceExtra`, so below an open Sequence view the box caught keys
  offset by the view's extra height. Landed in **TI-1**, not TI-2, with its regression
  test in `timeline_selection_test.dart`.

### 4.5 Snapping (completing docs/07 §4.5)

Sources and targets as spec'd. The still-unwired gestures — **bar drag, work-area
handles, marker drag, block stretch** — all route through `snapFrame` with the shared
target list, each drawing the capture line while a target holds the drag. `Ctrl` suspends
everywhere. The graph's key drags gain target snapping (markers, beat markers, playhead,
work-area edges) in the time axis, not only whole frames.

---

## 5. The keyframe mark — the seamless painter (the owner's finding *b*)

**Root cause.** `_LaneKeysPainter.paint` (timeline_panel_frb.dart ~9013) fills each mark
as **two separate paths** — `keyHalfPath(into, …, left: true)` then
`keyHalfPath(out, …, left: false)` — and Skia anti-aliases each path's edge against the
ground independently. Along the shared centre line both edges are half-covered, the
ground shows through the overlap of two 50% edges, and a hairline seam appears down the
middle of **every** key, same-shape pairs included.

**The drawing rule (binding).**

- A key's mark is painted with **one `drawPath` call**, from one path with **no interior
  edge on the centre line**.
- **Same-shape pairs** (`into == out`, hourglass excepted) return the whole shape's single
  closed contour — the diamond as one four-point polygon, the square as one rect.
- **Mixed pairs** return the union of the two halves as one contour: walk the left half's
  outer boundary from top-centre anticlockwise to bottom-centre, then the right half's
  outer boundary back up — the path crosses the centre line only at top and bottom (and at
  the pinch where an hourglass half meets it). Where a side's own shape touches the
  centre line along its full height (square, diamond meet at points; hourglass pinches),
  the touch is a point or the contour's own turn, never a drawn interior edge.
- The **hourglass/hourglass** pair keeps its two triangles: they meet at one point, and a
  point shared by two subpaths in one path draws no seam.
- `keyHalfPath` **stays** as the geometry oracle for tests; a new `keyMarkPath(pair, …)`
  composes the painted path. The regression test rasterises a same-shape pair at 4× and
  asserts the centre column's alpha equals its neighbours' (no lighter stripe), and the
  golden covers one mixed pair.
- The same painter serves Layers lanes, Keys mode and the shut layer's summary (K-459's
  one-painter rule).

---

## 6. Graph mode (the owner's finding *d*)

### 6.1 Handles

- **Handle lines are dashed** — the drawing's 2-on-2-off hairline in `text_primary` —
  never solid. Today `_HandlesPainter.paint` (graph_editor_frb.dart ~3017) draws solid
  `t.warning` lines; `warning` has no recorded job here. The endpoint dot becomes the
  drawing's **hollow ring**: `text_primary` stroke, `surface_0` fill
  (`_HandleDotPainter`, ~3090, draws a filled `warning` disc).
- **A dragged key carries its handles.** Root cause: `_shownKeys`
  (graph_editor_frb.dart ~2323) folds a row drag's and a handle drag's provisional
  geometry into the painted keys, but **not `_keyDrag`** — the value-lens key drag adds
  its delta only inside `_keyPoint` (~1902), so the key's glyph moves while
  `_handleEndpointFor`, `_tangentHandles` (~2684) and `_HandlesPainter` keep reading the
  document's unmoved keys: the line stretches from the moving key to a stranded endpoint
  and the endpoint dot never moves. The fix folds the key drag into `_shownKeys` (time
  and value deltas applied to every selected key, with the commit's own whole-frame
  rounding), so every reader — curve, endpoints, lines — derives from one moved list.
  Generalised in TI-7: a gesture in flight is a `_KeyMove` — *"where does the key at this
  frame and value go?"* — which both `_shownKeys` and the commit read, so the preview and
  the write cannot drift apart, and the transform box's scale rides the same rails as the
  key drag's delta. The
  regression test starts a key drag, moves it, and asserts the handle endpoint translated
  with the key before release.
- Breaking/joining stays `Alt` at drag start (shipped); the joined partner keeps its
  screen length (shipped). `Shift` lays the handle flat (shipped, K-333).
- Hovering a handle's target ring brightens the ring one step (P1); the cursor over key
  and handle targets is `move`/`resizeUpDown` respectively — today neither has a
  `MouseRegion` at all.

### 6.2 Keys and selection

- The graph's key glyphs keep their drawing: circle bezier, diamond linear, square hold
  (`GraphMode.dc.html` draws exactly this; the split-half hourglass is the *lanes'* mark —
  K-457/K-459 govern lanes and Keys mode, the graph's drawing governs the graph).
- A **selected** key draws in `text_primary`, one size step larger (the drawing's 7 in a
  6 world) — today `_keyHandles` paints selection in `t.accent`
  (graph_editor_frb.dart ~2668), which P4/K-439 forbid.
- While one key is selected or dragged, the **value hint pill** rides beside it:
  `f<frame> · <value> · <in> / <out> %` in 8px mono on `surface_4` — the drawing's pill.
  It follows a drag live and vanishes with the selection. **One** key: two or more are a
  block, and the block's own badge is the readout. It rides on the **value** lens, where
  the drawing puts it and where a key is one point; the speed lens draws a key as two dots
  with a speed each, which is a different readout and not this one.
- Box-select is additive with `Shift`/`Ctrl` (shipped) and restyles per §4.4.
- **The selection transform box** (docs/07 §5.3, Caddis §2.1) — **built in TI-7**: two or
  more selected keys in the value lens draw the same block box the lanes draw, spanning
  the selection in time *and value*; its left/right edges scale time about the opposite
  edge, its top/bottom edges scale value about the opposite edge. Same `text_primary`
  hairline, same one-undo commit, same Escape revert, same badge (`n keys · n f`), and
  the scaling arithmetic is the lane stretch's own (`scaledAbout`, `clampStretch` in
  `key_block.dart`) — time in frames, value in **pixels**, so that under Normalise, where
  each curve has its own range, one gesture still scales the whole selection by one
  amount. `Shift` **rounds what the scale lands on** — whole frames in time, whole
  numbers in value — with a readout pill under the box saying live what it reaches
  (`f<first>–f<last> · <low>–<high>`, gone on release, P1).
- **Two corrections to the sentence above, made in TI-7 and binding from here.**
  - **`Ctrl` does not taper.** docs/07 §5.3 named a taper on the corner drag; nothing
    draws it, no arithmetic for it is recorded anywhere, and `Ctrl` already has a job on
    every other drag in this panel — it suspends the magnet (§4.3, §4.5). `Shift` carries
    the modifier's work instead, which is what the study actually describes: *"Shift locks
    to the dominant drag axis and snaps values to integers with a live readout tooltip"*
    (`Caddis study/notes-editor-ux.md` §4). The axis lock half of that sentence belongs to
    the box's **slide** — the key drag, where `Shift` has constrained the axis since K-333
    — and the integer snap half belongs to the edge scale, which is where TI-7 put it.
    Logged as K-505.
  - **There are no corner grabs**, only the four edges. A box's corners stand exactly on
    the selection's extreme keys — with two keys selected they *are* those keys — so a
    corner grab would either swallow the key's own click and drag or sit unreachable
    underneath it, and P5 forbids both. The two axes are scaled in two gestures instead:
    the same arithmetic, one extra drag, and every key keeps every gesture it answers.
    The edge strips are 10px and sit **below** the key glyphs for the same reason.
- **Numeric entry** — **built in TI-7**: double-clicking a key opens a small popover
  holding its exact **frame, value, In % and Out %** (docs/07 §5.3, whose "speed" is not
  offered — a side's speed is what the tangent handle drags and what the influence field
  writes at, and a fourth number that restates it would be a second way to say one thing).
  Counted by timestamps (`DoubleTap`), never by an `onDoubleTap` recogniser, which would
  hold every single click on a key back until its timer expired. The frame field is
  bounded by the key's two neighbours, because the popover holds an index into a list a
  re-sort would shuffle; the channel is looked up by id at each write, never held, because
  a channel is a snapshot of the read model. The Key readout row of §3.3 covers the same
  numbers for the single-selection case without a gesture.

### 6.3 The tool strip and the frame

- The bottom strip gains **Tangents — Auto / Clamp / Free** between the lens pair and the
  ease presets (the drawing's strip; Caddis §2.1). Auto recomputes a smooth
  (Catmull-Rom-style) tangent whenever a neighbour moves, Clamp is Auto with the value
  clamped inside the neighbours (no overshoot), Free is today's behaviour. The mode is
  **per key side**, stored with the key; switching Free → Auto → Free keeps the custom
  ease (the study's explicit bar). This needs an engine/bridge seam (a tangent-mode field
  on `BridgeSideInterp`'s bezier arm or a sibling), designed inside its work package
  under docs/impl/keyframe-eval.md's maths.
- **Value labels live in a fixed right-hand gutter** (§12A.2; the drawing's 34px strip on
  a translucent ground — `graphGutterWidth`). Pinned to the **viewport's** right edge, not
  the canvas's: the pane is as wide as the whole composition inside the Timeline's
  horizontal scroll view, so a gutter fixed to the canvas would be off screen at every
  zoom but one. Drawn last, over the curves, so a curve can be seen running under its own
  numbers. Built in TI-6 (it used to pin them to the viewport's *left* edge).
- Curves already run edge to edge; the ruler, work area, cache bar and playhead are shared
  (shipped). Zoom/pan stays: wheel pans value (auto-fit off), `Alt`+wheel zooms value
  about the pointer, `Ctrl`/`Shift`+wheel keep the time bindings, `F` fits (shipped).
- Ease preset buttons, `F9` family, Linear/Bezier/Hold, the Easing panel, Vegas envelope:
  shipped; unchanged.

---

## 7. The ruler, work area and markers

Shipped: scrub on drag (previews video; stops playback per K-254), staged work-area edge
drags with the band's edges as handles, marker drag committing once on release, marker
right-click menu, one-per-frame replacement. Additions, each spec'd already and unbuilt:

- **Double-click the work-area band resets it to the whole comp** (docs/07 §4.1).
- **Double-click empty ruler creates a marker** at that frame and opens its label editor
  (docs/07 §4.1's "still to come").
- Work-area and marker drags **snap** (§4.5) with the capture line.
- The ruler ground shows no special cursor; the band's handles keep `resizeLeftRight`
  (shipped); a marker flag hovers to `click` (shipped) and brightens its pill one step.
- `B`/`N` set work-area start/end at the playhead (docs/07 §4.1 — keymap actions to add;
  both new keymap entries need their `engine_labels.dart` + arb pair, K-303).

---

## 8. The gap list

Each entry: tag, what, source, where. Bugs are behaviour that contradicts a shipped
claim; missing is drawn/ruled but absent; polish is the study's slickness bar.

### Bugs

1. **[bug] Lane keys do not travel with a block stretch.** Escaped string literal
   `'\${widget.rowId}#\$i'` in `_KeyLaneState._frameOf`. Source: K-458; the file's own
   comment promises the live travel. (§4.3) — **landed, TI-2.**
2. **[bug] Every keyframe mark draws a centre seam**, same-shape pairs included: two
   anti-aliased `drawPath` calls per mark in `_LaneKeysPainter.paint`
   (timeline_panel_frb.dart ~9013, geometry at `keyHalfPath` :159). Source: owner finding
   b; K-457. (§5)
3. **[bug] Graph handles lag a dragged key**: `_keyDrag` never enters `_shownKeys`
   (graph_editor_frb.dart ~2323), so `_handleEndpointFor`/`_tangentHandles`/
   `_HandlesPainter` read unmoved keys while `_keyPoint` (~1902) moves the glyph. Source:
   owner finding d. (§6.1)
4. **[bug] Keys mode's resting sheet takes no selection gesture**: layer bands are
   opaque tap-only (`_KeysLayerLane`, timeline_panel_frb.dart ~8396) and the shared twirl
   set opens shut (`_open`, :620; `keysLayerRows` reads it, :447), so the default sheet
   is all bands — no marquee, no property rows, no keys to click. Source: the Keys
   drawing; K-455/K-458. (§2.3)
5. **[bug] Marquee and block box misalign below an open Sequence view**: `_keysIn` /
   `_selectedKeyPlaces` step by `rowHeight` and ignore `sequenceExtra`
   (timeline_panel_frb.dart, `_LayerArea`). Source: K-248 heights vs K-458 walk. (§4.4)
   — **landed, TI-1.**
6. **[bug] Selection colours off-token**: graph selected key in `t.accent`
   (graph_editor_frb.dart ~2668); marquee box in `t.accent` (`widgets/marquee.dart`);
   handle lines/dots in `t.warning` (~3017, ~3090). Source: K-439/§3.1's closed accent
   list; the drawings select in `text_primary`. (§6.1, §6.2, §4.4)

### Missing

7. **[missing] Keys rows' anatomy** — layer number, stopwatch, navigator, value wells
   (K-499; `_KeysOutline` :6331, `_KeysLayerRow` :6436). Source: owner finding a. (§3.2)
   — **landed, TI-4.**
8. ~~**[missing] Graph mode's outline**~~ — **landed, TI-6**: the filtered colour-ticked
   list, Normalise, the Key readout row with In/Out % wells and the
   Layers-identical-outline setting. Source: K-442, §12A.2, GraphMode drawing. (§3.3)
9. **[missing] Dashed handle lines, hollow endpoint rings** (drawing). (§6.1)
10. **[missing] Value hint pill** beside a selected/dragged graph key, and the lane
    drag's `f · value` badge (drawing; study §3 "live readout"). (§4.2, §6.2)
11. **[missing] Tangents Auto / Clamp / Free** strip with per-side modes that survive a
    round-trip through Free (drawing strip; study §2.1). Engine seam required. (§6.3)
12. ~~**[missing] Graph selection transform box**~~ — **landed, TI-7**: the four edge
    grabs, time and value scaled about the opposite edge, `Shift` rounding what the scale
    lands on with its readout pill, the badge, one undo step and the Escape revert. No
    corner grabs and no `Ctrl` taper — both corrections recorded in §6.2 and logged as
    K-505. (§6.2)
13. ~~**[missing] Numeric entry**~~ — **landed, TI-7**: double-clicking a key opens its
    exact frame, value and In/Out % (docs/07 §5.3). (§6.2)
14. **[missing] Right-click menu on lane keys** (docs/07 §4.3; `_KeyLane` has no
    secondary-tap, ~8627). (§2.1)
15. **[missing] `Ctrl`+click plants a key on a lane** (docs/07 §4.3; graph only today).
16. **[missing] Additive (`Shift`) marquee in the lanes** — replace-only today
    (timeline_panel_frb.dart ~3361); the graph honours it (graph_editor_frb.dart :1620).
17. **[missing] Property-name click selects the property's keys** (K-500; today K-196
    selects the property only). (§2.1)
18. **[missing] Snapping for bar drags, work-area handles, marker drags and the block
    stretch**, each with the capture line (docs/07 §4.5's own still-to-build;
    timeline_extras_frb.dart work/marker drags commit unsnapped). (§4.5, §7) — **the
    block stretch landed in TI-2**; bar, work-area and marker drags stay with TI-9.
19. **[missing] Escape reverts any drag in flight** (study §2.2; P3). **Landed in TI-2**
    for the bar, the lane key and the block stretch, on one shared `DragEscape`
    (`widgets/drag_escape.dart`); **the graph's three drags — key, tangent handle and
    transform box — adopted it in TI-7**, on one `DragEscape` the state holds, since only
    one of them can be in flight at a time. The marker and work-area drags stay with
    TI-9.
20. **[missing] Double-click resets the work area / creates a marker** (docs/07 §4.1;
    ruler has no double-tap, timeline_extras_frb.dart ~1253).
21. **[missing] `B`/`N` work-area keymap actions** (docs/07 §4.1; no such actions in the
    keymap). Each needs its engine label + arb pair (K-303).
22. ~~**[missing] Value labels in a fixed right-hand gutter**~~ — **landed, TI-6**
    (§12A.2). (§6.3)
23. **[missing] Edge-follow scrolling during playback; `=`/`-`/`\` zoom keys**
    (docs/07 §4.6 still-to-build).
24. **[missing] Acceleration lens, auto view, beat-marker lines in the graph, waveform
    ghosting, Retime lenses** (docs/07 §5.1–5.3 still-to-build — recorded here for
    completeness; they stay on their existing TODO lines and are not in this
    programme's packages).

### Polish

25. **[polish] Cursor vocabulary**: move cursor on a bar's body, `move` on graph keys,
    `resizeUpDown` on handle rings — today only bar ends, lane keys, block handles and
    work-area edges set one (study §9: "the cursor shape is doing most of the affordance
    work"). (§4.1, §6.1)
26. **[polish] Hover states**: key brightens at half strength, bar firms its leading
    edge, marker pill and handle ring brighten one step — all P1-transient, nothing at
    rest (study §11/§12). Rows already wash on hover.
27. **[polish] Drag-scrub modifier ladder hint** — the floating
    `CTRL ×0.1 · BASE · SHIFT ×10` chip shown only while a value scrub runs, active level
    boxed (study §2.2, PLAN A2). Shared with Effect controls; listed here because the
    fold rows scrub too.
28. **[polish] Marquee/block visuals to the drawings** — `text_primary` hairline box, the
    12% wash (see bug 6). — **landed, TI-2.**

---

## 9. Work packages

One agent each, in order; every package lands with its tests (regression tests named in
the sections above) and keeps `flutter analyze` clean. **All of these run after the
concurrent export-workflow session has landed its commits** — it works in the same files
(`timeline_panel_frb.dart`, `timeline_extras_frb.dart`, the l10n arbs); rebase on its
result rather than merging around it. New user-facing strings (menu rows, keymap labels,
the badge) land in `app_en.arb` in the same commit, with `engine_labels.dart` entries for
anything the engine names (K-303, K-005); PRs list the new keys for Crowdin.

- **TI-1 — Selection reaches everywhere** (§2; gaps 4, 14–17, and bug 4). Marquee from
  any ground (Keys bands pass drags through; bar rows admit the box off the bar),
  additive `Shift`/`Ctrl` marquee in the lanes, Keys mode opening twirled open with its
  own default set, property-name selects its keys (`Ctrl`/`Shift` variants), lane key
  right-click menu, `Ctrl`+click plants a lane key. Files: `timeline_panel_frb.dart`,
  `widgets/marquee.dart`; tests beside the existing timeline widget tests; arb keys for
  the menu.
- **TI-2 — The block feel** (bugs 1, 5; gaps 18–19 for the stretch; polish 28). The
  escaped-string fix with its live-travel regression test; the `sequenceExtra` walk fix;
  stretch snapping with the capture line; Escape reverting stretch, lane-key and bar
  drags; marquee/block colours to `text_primary`. Files: `timeline_panel_frb.dart`,
  `key_block.dart`, `widgets/marquee.dart`.
- **TI-3 — The seamless key mark** (§5; bug 2). `keyMarkPath`, the one-`drawPath`
  painter, the centre-column raster test and mixed-pair golden. Files:
  `timeline_panel_frb.dart` (painter + `keyHalfPath` oracle), a paint test.
- **TI-4 — Keys rows to K-499** (§3.2; gap 7). **Landed.** Layer number, stopwatch,
  navigator and value wells on the Keys property rows via the shared row machinery —
  by listing `_FoldRow` itself. Files:
  `timeline_panel_frb.dart` (`_KeysOutline`, `_KeysLayerRow`),
  `keyframe_controls_frb.dart` reuse; widget tests pinning the anatomy.
- **TI-5 — Graph handles and drag geometry** (§6.1–6.2; bugs 3, 6; gaps 9–10). Dashed
  `text_primary` lines, hollow rings, `_keyDrag` folded into `_shownKeys` with the
  handle-travel regression test, selected key in `text_primary` one step larger, the
  value hint pill (graph and lane badge), hover/cursor on keys and rings. Files:
  `graph_editor_frb.dart`, `timeline_panel_frb.dart` (lane badge).
- **TI-6 — Graph mode's drawn surface** (§3.3, §6.3; gaps 8, 22). **Landed.** The
  filtered colour-ticked outline with Normalise and the Key readout row (In/Out wells),
  the right-hand value gutter, the Layers-identical-outline setting. Files:
  `timeline_panel_frb.dart`, `graph_editor_frb.dart`, `settings.dart`,
  `settings_window_frb.dart`, arb keys.
- **TI-7 — Graph transform box and numeric entry** (§6.2; gaps 12–13). **Landed.** The
  selection box with **edge** scaling of time and of value about the opposite edge,
  `Shift` rounding the result with its live readout, sharing TI-2's escape and one-undo
  behaviour; the double-click exact-fields popover. No corner grabs, no `Ctrl` taper
  (§6.2, K-505). Files: `graph_editor_frb.dart`, `key_block.dart` (`scaledAbout`, the
  shared block maths), `timeline_panel_frb.dart` (the Key readout row's influence write
  moved to the shared `sideWithInfluence`), arb keys.
- **TI-8 — Tangent modes** (§6.3; gap 11). The per-side Auto/Clamp/Free field through
  engine, bridge and strip, with the Free-round-trip-keeps-the-ease test; the maths lands
  in docs/impl/keyframe-eval.md in the same commit. Files: `crates/lumit-core` (keyframe
  store), `crates/lumit-bridge`, codegen, `graph_editor_frb.dart`. Runs **after TI-5**.
- **TI-9 — Snapping and ruler completion** (§4.5, §7; gaps 18, 20–21, 23). Bar/work-area/
  marker snapping with capture lines, double-click reset/create, `B`/`N` actions (engine
  labels + arb), edge-follow during playback, `=`/`-`/`\`. Files:
  `timeline_panel_frb.dart`, `timeline_extras_frb.dart`, `timeline_snap.dart`,
  `crates/lumit-bridge/src/api/keymap.rs`.
- **TI-10 — The hover, cursor and hint pass** (polish 25–27). The cursor table, the
  hover ladder, the scrub modifier-ladder chip (shared widget, wired here for the
  timeline's rows; Effect controls adopts it in its own panel work). Files:
  `timeline_panel_frb.dart`, `widgets/` (the chip), tests asserting the `MouseRegion`
  cursors. Runs last — it touches every surface the earlier packages settle.

TI-1/TI-2/TI-3 are the owner's reported symptoms and go first; TI-5 unblocks TI-7/TI-8;
TI-4 and TI-6 are independent of each other.

## Open questions

- Should double-click on empty graph curve also plant a key (Caddis's gesture), or does
  `Ctrl`+click stay the only planting gesture (docs/07 §4.3's, shipped)? Leaning: keep
  `Ctrl`+click only, since double-click is being given to numeric entry on a key and two
  double-click meanings a few pixels apart misfire.
- ~~Whether the Keys-mode value wells write through the playhead edit rule K-189
  unchanged~~ — **closed in TI-4, by test rather than by ruling.** They do, because they
  are the fold-out's own wells: a typed value lands on the key under the playhead,
  flattening nothing and planting nothing
  (`timeline_panel_frb_test.dart`, "a Keys value well writes into the key at the
  playhead").
