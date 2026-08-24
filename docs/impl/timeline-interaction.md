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
  the drag starts **adds** to the standing selection (the graph's `_applyMarquee` already
  honours this; the lanes must match).
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
- `Ctrl`+click on empty lane space of a **keyed** row plants a key at that time on that
  property (docs/07 §4.3's sentence, unbuilt in the lanes; the graph ships it).

### 2.2 Deselection and scope

- Selection lets go exactly as K-203 says: closing a fold drops what was inside it,
  selecting a layer clears the property selection, empty-ground click clears all.
- The lane, Keys and graph key selections are one selection wherever the id space allows
  (a lane key fans out to its axes' channels — shipped, `_actionKeySelection`).

### 2.3 Keys mode's ground

- **Keys mode opens with every listed layer twirled open.** The dope sheet's point is the
  flattened property rows; a sheet of shut bands shows nothing and takes no marquee. The
  Keys twirl set starts as "all open" (its own default, still toggleable per layer) rather
  than inheriting Layers mode's shut-by-default set.
- A Keys-mode **layer band selects on tap but never swallows a drag**: drags pass through
  to the marquee. (Today `_KeysLayerLane` is an opaque `GestureDetector`,
  timeline_panel_frb.dart ~8396, and eats both.)

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
  `keyframe_controls_frb.dart` control) · **◄ ◆ ► navigator** (drawn once animating, the
  reserved slot rule of the effect controls) · the name as the drawing writes it —
  `Group · Name`, group muted, a middle dot between — · the **value in an editable well**,
  scrub-drag and click-to-type, the same `transform_rows_frb.dart` /
  `effect_param_row_frb.dart` machinery the Layers fold-out and Effect controls share, so
  a parameter behaves the same wherever it is shown (docs/07 §4.3's one-implementation
  rule). The resting value text keeps `animated` — on this sheet every listed row is
  keyed — and the unit stays outside the well.
- The property-name click follows §2.1 (selects the property and its keys); the value
  well and stopwatch never re-aim the selection (K-196's rule, unchanged).
- The rows stay flat (no group headings) and the filters row stays as built.

### 3.3 Graph mode

The Graph outline is **not built to its drawing** (today it shows the full Layers columns;
timeline_panel_frb.dart ~2983 chooses `_Outline` whenever `_keys` is false). Per K-442,
§12A.2 and `GraphMode.dc.html` it must become:

- The **filtered animated list**: Show — Animated (default) / All, no Selected; one row
  per animated property, carrying an **include-in-graph tick** and a **swatch in the
  curve's colour**, the name (`Position · X` style, the axis split out), and the value at
  the playhead — `animated` when the row is shown in the graph, muted when unticked.
- A **Normalise** checkbox at the row's far right of the filter row (drawn; behaviour:
  each shown curve scaled to its own min–max so unlike units share the pane — a view
  setting, never data).
- A **Key readout row** pinned at the outline's foot while exactly one key is selected:
  `KEY f<frame> <value><unit>` then `In [well] % Out [well] %` — the influence wells
  editable, committing through the same tangent write the handles use.
- A setting restores an outline identical to Layers mode (K-442, unbuilt).

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
  once (all shipped); **Escape reverts** (new, P3).
- A hovered key brightens to `text_primary` at half strength — the pre-selection hint —
  and its time/value appear nowhere until the drag starts (P1).
- **While a key drag or block stretch runs, a badge under the pointer reads
  `f<frame> · <value>`** (frame only for a multi-key stretch: `f<first>–f<last>`), in the
  block badge's own 8px mono on `surface_4`. This is the drawing's value-hint pill and the
  study's "live readout"; it vanishes on release.
- A shut layer's **summary diamonds** stay a statement, not a target (K-441) — but the row
  they sit on admits the marquee everywhere the bar itself is not (§2.1).

### 4.3 The block tools (K-458)

Shipped: the box, the 3×6 end marks in 11px targets, the badge opening the Ease popover,
whole-frame stretch, one undo step, Reverse / Copy / Paste-at-playhead in the Keys strip.
Corrections and additions:

- **The keys must travel with the stretch.** They do not: `_KeyLaneState._frameOf`
  compares the stretch's key set against the **escaped literal** `'\${widget.rowId}#\$i'`
  (timeline_panel_frb.dart:8503) — backslashed dollars, so the string is never
  interpolated, the `contains` never matches, and every diamond sits still until release
  while the box moves. The fix is the two backslashes; the regression test drags a handle
  and asserts a covered key's drawn frame moved before release.
- The stretch handle snaps its dragged end to the shared targets (today: whole frames
  only), same capture line, `Ctrl` suspends.
- The badge count/span updates live through the stretch (shipped) and the whole gesture
  reverts on Escape (new).
- The box, marks and badge follow the selection into **Layers mode** identically (shipped
  — one `_LayerArea` draws both modes).

### 4.4 The marquee

- Start conditions per §2.1 — any ground, including Keys-mode bands and bar rows off the
  bar. Additive with `Shift`/`Ctrl` held at drag start.
- The box draws a `text_primary` hairline with a faint wash — **not the accent**
  (`widgets/marquee.dart` draws `t.accent` today; P4). The graph's marquee restyles the
  same way, one widget.
- The catch walk must step by each layer's **real block height**: `_keysIn` and
  `_selectedKeyPlaces` (timeline_panel_frb.dart, in `_LayerArea`) advance by `rowHeight`
  per row and ignore `sequenceExtra`, so below an open Sequence view the box catches keys
  offset by the view's extra height. Walk `row.height` per block, as the drag-slot maths
  already does.

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
  and value deltas applied to every selected key, the same `_snappedDx` rounding the dot
  uses), so every reader — curve, endpoints, lines — derives from one moved list. The
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
  It follows a drag live and vanishes with the selection.
- Box-select is additive with `Shift`/`Ctrl` (shipped) and restyles per §4.4.
- **The selection transform box** (docs/07 §5.3, Caddis §2.1): two or more selected keys
  in the value lens draw the same block box the lanes draw, spanning the selection in
  time *and value*; its left/right edges scale time about the opposite edge, its
  top/bottom edges scale value, corners scale both, `Ctrl` tapers. Same `text_primary`
  hairline, same one-undo commit, same Escape revert, same badge (`n keys · n f`).
- **Numeric entry**: double-clicking a key opens exact time / value / influence fields
  (docs/07 §5.3, still to build — the Key readout row of §3.3 covers the single-selection
  case; the double-click editor is the popover form).

### 6.3 The tool strip and the frame

- The bottom strip gains **Tangents — Auto / Clamp / Free** between the lens pair and the
  ease presets (the drawing's strip; Caddis §2.1). Auto recomputes a smooth
  (Catmull-Rom-style) tangent whenever a neighbour moves, Clamp is Auto with the value
  clamped inside the neighbours (no overshoot), Free is today's behaviour. The mode is
  **per key side**, stored with the key; switching Free → Auto → Free keeps the custom
  ease (the study's explicit bar). This needs an engine/bridge seam (a tangent-mode field
  on `BridgeSideInterp`'s bezier arm or a sibling), designed inside its work package
  under docs/impl/keyframe-eval.md's maths.
- **Value labels move to a fixed right-hand gutter** (§12A.2; the drawing's 34px strip on
  a translucent ground). Today `_GraphPainter` pins them to the viewport's left edge.
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
   `'\${widget.rowId}#\$i'` in `_KeyLaneState._frameOf` —
   `flutter_ui/lib/panels/timeline_panel_frb.dart:8503`. Source: K-458; the file's own
   comment promises the live travel. (§4.3)
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
6. **[bug] Selection colours off-token**: graph selected key in `t.accent`
   (graph_editor_frb.dart ~2668); marquee box in `t.accent` (`widgets/marquee.dart`);
   handle lines/dots in `t.warning` (~3017, ~3090). Source: K-439/§3.1's closed accent
   list; the drawings select in `text_primary`. (§6.1, §6.2, §4.4)

### Missing

7. **[missing] Keys rows' anatomy** — layer number, stopwatch, navigator, value wells
   (K-499; `_KeysOutline` :6331, `_KeysLayerRow` :6436). Source: owner finding a. (§3.2)
8. **[missing] Graph mode's outline** — filtered colour-ticked list, Normalise, the Key
   readout row with In/Out % wells, the Layers-identical-outline setting
   (timeline_panel_frb.dart ~2983 always takes `_Outline`). Source: K-442, §12A.2,
   GraphMode drawing. (§3.3)
9. **[missing] Dashed handle lines, hollow endpoint rings** (drawing). (§6.1)
10. **[missing] Value hint pill** beside a selected/dragged graph key, and the lane
    drag's `f · value` badge (drawing; study §3 "live readout"). (§4.2, §6.2)
11. **[missing] Tangents Auto / Clamp / Free** strip with per-side modes that survive a
    round-trip through Free (drawing strip; study §2.1). Engine seam required. (§6.3)
12. **[missing] Graph selection transform box** with edge-drag time/value scaling and
    `Ctrl` taper (docs/07 §5.3 still-to-build; study §2.1). (§6.2)
13. **[missing] Numeric entry** — double-click a key for exact fields (docs/07 §5.3).
14. **[missing] Right-click menu on lane keys** (docs/07 §4.3; `_KeyLane` has no
    secondary-tap, ~8627). (§2.1)
15. **[missing] `Ctrl`+click plants a key on a lane** (docs/07 §4.3; graph only today).
16. **[missing] Additive (`Shift`) marquee in the lanes** — replace-only today
    (timeline_panel_frb.dart ~3361); the graph honours it (graph_editor_frb.dart :1620).
17. **[missing] Property-name click selects the property's keys** (K-500; today K-196
    selects the property only). (§2.1)
18. **[missing] Snapping for bar drags, work-area handles, marker drags and the block
    stretch**, each with the capture line (docs/07 §4.5's own still-to-build;
    timeline_extras_frb.dart work/marker drags commit unsnapped). (§4.5, §7)
19. **[missing] Escape reverts any drag in flight** (study §2.2; P3) — bar, lane key,
    stretch, marker, work-area, graph drags all lack it.
20. **[missing] Double-click resets the work area / creates a marker** (docs/07 §4.1;
    ruler has no double-tap, timeline_extras_frb.dart ~1253).
21. **[missing] `B`/`N` work-area keymap actions** (docs/07 §4.1; no such actions in the
    keymap). Each needs its engine label + arb pair (K-303).
22. **[missing] Value labels in a fixed right-hand gutter** (§12A.2; painter pins left,
    graph_editor_frb.dart ~2466). (§6.3)
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
    12% wash (see bug 6).

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
- **TI-4 — Keys rows to K-499** (§3.2; gap 7). Layer number, stopwatch, navigator and
  value wells on the Keys property rows via the shared row machinery. Files:
  `timeline_panel_frb.dart` (`_KeysOutline`, `_KeysLayerRow`),
  `keyframe_controls_frb.dart` reuse; widget tests pinning the anatomy.
- **TI-5 — Graph handles and drag geometry** (§6.1–6.2; bugs 3, 6; gaps 9–10). Dashed
  `text_primary` lines, hollow rings, `_keyDrag` folded into `_shownKeys` with the
  handle-travel regression test, selected key in `text_primary` one step larger, the
  value hint pill (graph and lane badge), hover/cursor on keys and rings. Files:
  `graph_editor_frb.dart`, `timeline_panel_frb.dart` (lane badge).
- **TI-6 — Graph mode's drawn surface** (§3.3, §6.3; gaps 8, 22). The filtered
  colour-ticked outline with Normalise and the Key readout row (In/Out wells), the
  right-hand value gutter, the Layers-identical-outline setting. Files:
  `timeline_panel_frb.dart`, `graph_editor_frb.dart`, settings page, arb keys.
- **TI-7 — Graph transform box and numeric entry** (§6.2; gaps 12–13). The selection box
  with edge/corner scaling and `Ctrl` taper, sharing TI-2's escape/undo/snap behaviour;
  the double-click exact-fields editor. Files: `graph_editor_frb.dart`, `key_block.dart`
  (shared block maths).
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
- Normalise's exact scaling (per-curve min–max vs shared symmetric range) — decide inside
  TI-6 with the drawing open.
- Whether the Keys-mode value wells write through the playhead edit rule K-189 unchanged
  (they should: same machinery) — confirm with a test in TI-4 rather than a ruling.
