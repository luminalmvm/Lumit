# The grey room — complete surface inventory

**What this is.** Every surface, control and state Lumit draws, as [07-UI-SPEC.md](../07-UI-SPEC.md)
and the shipping frontend list them, with how the grey room draws each one. It exists to be
walked top to bottom, one line at a time, so that nothing the application does is left without
a ruling before anything is built. The rulings themselves are argued in
[15-DESIGN-RAMS.md](15-DESIGN-RAMS.md); this document only applies them.

**How to read a line.** Every item is a checkbox with a status:

- **drawn** — it is on [mockups/rams-shell.html](mockups/rams-shell.html) and can be judged by eye.
- **spec** — the Rams document rules on it, but the mockup does not draw it.
- **derived** — no explicit ruling; the treatment follows mechanically from the system (the module,
  the label, the well, the signal list). Stated here so it can be disagreed with.
- **open** — a decision is needed before it can be built. These are gathered again at the end.

Tick a box when the line has been read and either accepted or annotated. A line that turns out
to be wrong is corrected in the Rams document, not here.

**The system in one paragraph**, so the lines below can be short. Two rooms (grey, graphite);
one face (IBM Plex Sans, tabular figures); one 9px lowercase muted **label** for every container;
one **well** (inset on `surface_0`, 20 tall) for every editable value; one **signal** colour for
now-and-in-hand; three state marks (on / attention / fault); every height a multiple of 4 and
one of 16 / 20 / 24 / 28 / 32 / 48; radius 2, full only for a true dial; hairlines for elevation
and a 3px-blur shadow only on the closed float list; hover and press are surface steps, never
strokes or hues; nothing animates at rest.

---

## 0. The application shell

### 0.1 The window

- [ ] **Window chrome** — opens maximised, remembers geometry; nothing to draw. *derived*
- [ ] **Application background** — `surface_0`, the grey room's `#dedcd8`. The dock's 1px seams are `hairline`. *drawn*
- [ ] **UI scale setting** (100 / 125 / 150%) — the module scales with it; 4 stays 4 logical px. *derived*

### 0.2 The top line (28)

- [ ] **The mark** — twin keyframes, flat colours, at 16 in a 44×28 cell. *drawn*
- [ ] **Menu bar, in-window** (Windows/Linux) — nine lowercase words, 11px `text_secondary`, 14 apart. *drawn*
- [ ] **Menu bar, macOS** — the system bar; the line keeps mark, workspaces, palette well. *spec*
- [ ] **Workspace strip** — lowercase names, the fronted one `text_primary` over a 2px `signal` rule. `Alt+Shift+1…9`. *drawn*
- [ ] **Workspace strip overflow** on narrow windows — trailing names fold into one overflow mark (the canonical ladder step 4). *derived*
- [ ] **Command palette well** — a 200×20 ghost well at the right with the magnifier and the chord as a `text_disabled` hint. *drawn*
- [ ] **Update state in Help ▸ Check for updates** — the row's text changes in place (checking… / click to update / restart to finish); no colour. *derived*

### 0.3 The tool rail (44 wide, full height)

- [ ] **Rail ground** — `surface_1`, 1px `hairline` on its right. *drawn*
- [ ] **A tool cell** — 44×44, glyph at 16 in `text_secondary`; hover `surface_3`; pressed `hairline_strong`. *drawn*
- [ ] **The armed tool** — `surface_2` cell with a 2px `signal` index on the rail's edge, glyph `text_primary`. *drawn*
- [ ] **A group cell** — carries the member last used and a 4px corner triangle in `text_muted`. *drawn*
- [ ] **A group's flyout** — opens to the right of the rail on `surface_3`, 3px-blur shadow, one 24px row per member with glyph, name and chord; the unbuilt drawn `text_disabled`. *spec*
- [ ] **A disabled tool** — glyph `text_disabled`; tooltip says it is not built. *drawn* (Puppet and Camera drawn live in the mockup for illustration; disabled treatment is the same cell dimmed)
- [ ] **The seam between the six simple tools and the seven groups** — 1px `hairline`, 10 inset. *drawn*
- [ ] **Snapping switch** — at the foot of the rail; `text_primary` on, `text_muted` off. *drawn*
- [ ] **Tool options panel** — hangs off the rail beside the armed tool while a tool with options is armed: a `surface_1` card, hairline-bounded, one 24px row per option (fill swatch, size well, hardness, opacity, stroke swatch, stroke width, density, expansion, roto size). Selection shows none. *spec*
- [ ] **Tool tooltips** — name and chord, one line. *derived*
- [ ] **Keyboard focus on a rail cell** — the 1px `signal` ring, 1px outside. *derived*

### 0.4 The dock

- [ ] **Frames and seams** — panes butt together on 1px `hairline`; no gaps, no inset. *drawn*
- [ ] **Bare pane** — the default: no tab bar; the pane's label engraved in its top-left corner on a 24px line with the pane's own controls. *drawn*
- [ ] **A tab group** (two or more panels stacked) — a 24px strip of lowercase tab names; the fronted one `text_primary`, the rest `text_muted`; no fill, no outline, no rule (the canonical panel-tab ruling). *spec*
- [ ] **Panel menu** (top-right of a group: undock, close, maximise, help) — a bare glyph on the strip. *derived*
- [ ] **Drop zones during a panel drag** — five zones, each a 1.5px dashed `signal` outline with `signal` @10% fill; the pending layout previewed as a hairline outline. *spec*
- [ ] **Panel drag ghost** — the pane's header strip alone, following the pointer, with the 3px-blur shadow; pins to the cursor, no lag. *spec*
- [ ] **Floating window** (undocked panel) — a real OS window on `surface_1` with its own dock tree; no in-app shadow. *derived*
- [ ] **Maximise a panel** (backtick) — the pane fills the dock; nothing else changes. *derived*
- [ ] **Panel minimum widths** — declared once, enforced by the seam and the slide. *derived*
- [ ] **Width degradation ladder** — ellipsise flexible text → wrap control runs → hide optional columns → overflow toolbars → scroll. Unchanged. *spec*

### 0.5 The readout strip (20)

- [ ] **Ground** — `surface_1`, 1px `hairline` above; all text 9px. *drawn*
- [ ] **Status message** — `text_secondary` at the left: "Lumit is up to date", "Exporting — 41% · 02:12 remaining", "Making proxy — 30%", with **Cancel** as a lowercase word beside a cancellable job. *drawn* (idle state)
- [ ] **Progress in the status line** — a 48×3 bar in `hairline_strong` beside the message, filling; no colour. *spec*
- [ ] **Cache meters** — vram / ram / disk, each a label, a 48×3 bar (`surface_0` track, `hairline_strong` fill) and the figure. *drawn*
- [ ] **Frame cost** and **dropped frames** — figures at the right; dropped frames is information, `text_muted`. *drawn*
- [ ] **Engine refusals** (measuring refused, tier unavailable) — one calm sentence in the message slot. *derived*
- [ ] **Measuring switch** — the clock glyph lives on the Timeline outline's foot (§4.9), not here. *drawn*

---

## 1. The Viewer

### 1.1 The bar above the picture (24) — ways of looking

- [ ] **Label** `viewer`. *drawn*
- [ ] **Magnification picker** — a 10px word with a 8px chevron (`fit 62%`); menu on `surface_3`. *drawn*
- [ ] **Preview resolution picker** — `half`; its menu also carries the playback behaviour (adaptive / every frame) as option rows that leave the menu open. *drawn*
- [ ] **Channel mark** — the one glyph with colour of its own: tri-colour for RGB, one circle for R/G/B, near-white for alpha. Not boxed. *drawn*
- [ ] **Transparency board toggle** — the checkerboard glyph. *drawn*
- [ ] **View menu mark** — one glyph; the menu carries wireframes, motion paths, mask paths, gizmo visibility, layer-controls switch, region of interest, grid, title/action safe, rulers, snap to guides, clear guides, composition background (with swatch), full wireframe mode. Ticked rows use the set's tick. *drawn* (mark); *spec* (menu)
- [ ] **3D view picker** — `active camera` and the fixed/custom views. *drawn*
- [ ] **Exposure** — the number alone, 10px tabular, `text_secondary`; the reset mark appears to its left only while non-zero. *drawn*
- [ ] **Snapshot pair** — Take and Show behind a seam; Show `text_disabled` until a snapshot exists. *drawn*
- [ ] **Colour pipeline picker** — `sRGB`; its menu carries the tone-map row when Settings asks for it. *drawn*
- [ ] **Degradation reading** — `1920×1080 → 960×540` in a fixed-width slot, 9px `text_muted`. *drawn*
- [ ] **Bar overflow** — collapses from the right into a chevron menu. *derived*
- [ ] **Nothing on the bar moves as the picture changes** — every varying part sits in a slot sized for its longest value. *derived*

### 1.2 The stage

- [ ] **Surround** — `#7f7f7f`, identical in both rooms; the user's slider offers neutral greys only; the themed-surround opt-in survives, off by default. *drawn*
- [ ] **Transparency board** — two neutral greys, clipped to the picture, costing the panel not the picture. *derived*
- [ ] **Letterbox bars** — surround grey. *derived*
- [ ] **Neutrality zone** — nothing saturated within 48px of the picture except overlays on the picture itself. *drawn*
- [ ] **Selection name on the picture** — 9px `signal` word in a `signal` hairline, 16 in, 8 down. *drawn*
- [ ] **"At effect" chip** — top-right over the picture, `surface_1` card, hairline, lowercase `at gaussian blur`; reads engaged while on. *drawn*
- [ ] **Preview progress bar** — on the deck's right end, 48×3, `hairline_strong` fill with a slow sheen; appears only after ~150ms, never during playback. *spec*
- [ ] **Rulers** (top, left, 18 tall) — `surface_2` bands, hairline seam, 9px muted labels on a 1/2/5 ladder. *derived*
- [ ] **Guides** — 1px `signal` lines (a tool on the picture), with a thin grab strip. *derived*
- [ ] **Grid (eighths)** and **title/action safe** — `hairline` lines over the picture. *derived*
- [ ] **Region of interest outline** — 1px dashed `text_secondary` while in force; a small × to clear. *derived*
- [ ] **Snapshot shown (held)** — the stored picture over the live one, no border, no badge. *derived*
- [ ] **Footage mode source-time strip** and **Layer mode** — a 24px strip under the picture in `surface_2` with in/out wells and a slip readout. *open* — the strip's anatomy is not drawn anywhere yet
- [ ] **Multi-view layouts** (several views in one Viewer) — each view a stage with a 1px `hairline` seam; the active view marked by a 2px `signal` rule under its own strip. *open* — rule proposed here, not in the Rams doc
- [ ] **View lock padlock** — a bare glyph on the view's strip; `text_primary` locked. *derived*
- [ ] **Empty Viewer** (no composition) — the welcome's two start cards centred, capped at 560. *spec*
- [ ] **"Select a composition" line** — one sentence, `text_muted`, centred. *derived*

### 1.3 Layer controls and overlays (on the picture, so colour is allowed)

- [ ] **Wireframe** of a selected layer — 1px `signal`. *drawn*
- [ ] **Hover box** of an unselected layer — 1px `signal` @40%. *derived*
- [ ] **Scale handles** — 6×6 `signal` squares at corners and edge midpoints. *drawn*
- [ ] **Rotation bar** — a 1px `signal` line standing 18 off the top edge. *drawn*
- [ ] **Anchor handle** — an 8px `signal` ring at the centre. *drawn*
- [ ] **Marquee** — 1px dashed `signal` with @10% fill. *derived*
- [ ] **Snap indication** at the moment of snap — a 1px `signal` hairline at what caught the drag. *derived*
- [ ] **"Neutral handles" option** — every overlay above in `text_secondary` grey instead. *derived*
- [ ] **Motion path** — 1px `text_primary` path, per-frame dots at 2px, keyframe boxes 6px, spatial handles as `text_muted` stems. *derived*
- [ ] **Mask paths** — 1px `signal`; vertices 5px squares; feather handles as stems. *derived*
- [ ] **Shape in flight** (shape/pen tools) — the path as it is drawn, 1px `signal`. *derived*
- [ ] **Type edit** — caret and selection in the layer's own text; no chrome. *derived*
- [ ] **Paint stroke in flight** — the stroke itself; brush ring per §1.4. *derived*
- [ ] **Clone source mark** — a 12px `signal` crosshair. *derived*
- [ ] **Roto overlay** — subject in `on`, background in `fault`, refine in `signal`, faded to 60%; Boundary view draws the matte edge with a halo. *derived*
- [ ] **Camera gizmo and orbit circle** — 1px `signal`. *derived*
- [ ] **Camera-track point cloud** — depth-cued dots in `text_primary`, nearer larger and stronger. *derived*
- [ ] **Point-cloud selection** — picked dots in `signal`; the floating **Create null / Create solid** row is a `surface_1` card, hairline, two lowercase actions, 24 tall. *derived*
- [ ] **Puppet pins** — 8px rings in `signal`, starch/overlap/bend distinguished by glyph. *derived*
- [ ] **Planar track display** — the tracked quad in `signal`, corner marks 6px. *derived*
- [ ] **Compare / wipe** (viewer_compare) — a 1px `text_primary` divider with a 12px grab. *open* — not specified anywhere yet

### 1.4 The tools' pointers

- [ ] **Hardware crosshair** for aiming tools, badged down-right with the tool glyph on a halo. *derived*
- [ ] **Brush ring** at brush size in picture scale, 1px `text_primary` with a 1px dark halo. *derived*
- [ ] **Drawn pointers** — Hand (open / closed), Zoom (magnifier with ±), Rotation (curved arrow, eight positions), Anchor point (ring and gapped cross), vertical I-beam. All 1.25px strokes, `text_primary` on a halo. *derived*
- [ ] **Razor line** in the Timeline — a 1px `text_primary` line across every lane at the frame it would cut. *derived*
- [ ] **Dropper magnifier** — a 9×9 grid card on `surface_3` with dashed rules, a 1px `text_primary` border round the sampled region (2px corners), and the reading strip under it. *derived*
- [ ] **Armed dropper glyph** — lit `signal` while armed. *derived*

### 1.5 The deck (32) — playing

- [ ] **Ground** — `surface_2`, 1px `hairline` above. *drawn*
- [ ] **Transport** — five 16px glyphs in `text_primary`, 12 apart: to start, previous frame, play/pause, next frame, to end. *drawn*
- [ ] **Clock** — 15px tabular `text_primary`, click to type, drag to scrub; the one large number. *drawn*
- [ ] **Frame count** — `f112 /525`, 10px `text_muted` beside the clock; edits as the bare number. *drawn*
- [ ] **Loop mode picker** — `loop` / play once / ping-pong. *drawn*
- [ ] **Preview mode picker** — `cached` / realtime. *drawn*
- [ ] **Quality toggle** — `full` / draft. *drawn*
- [ ] **Audio mute** — speaker glyph; muted draws the set's muted speaker. *drawn*
- [ ] **Cache-ready meter** — label, 60×4 bar with `cache_hot` fill, percentage; skipped frames stated after it while playing uncached. *drawn*
- [ ] **Fill cache action** — a lowercase word after the meter. *spec*
- [ ] **Preview progress bar** — the right-hand end (§1.2). *spec*
- [ ] **Dockable Preview panel** — the same controls as a pane when the user wants a second copy. *derived*

---

## 2. The Project panel

- [ ] **Label** `project` and, at the right, the count `10 items · 1 missing` (the missing half is the filter). *drawn*
- [ ] **Specimen card** (75) — 96×54 poster, and a two-column engraved readout: name, size · rate, length, codec, sound. Hover-scrubs the poster on a clip; a sound file shows a play glyph in the square. *drawn* (readout); *derived* (scrub, play glyph)
- [ ] **A still says `still`** where a rate and a length would be. *derived*
- [ ] **Search well** (32 row, 20 well) with the **colour-swatch filter** as a 10px square in the leading slot; the square wears the held colour or the four-hue quartering. *drawn*
- [ ] **Column headers** — lowercase labels: name · items · size · fps · path; ruled seams; the flexible one is name. *drawn*
- [ ] **Rows** (24) — glyph tinted by the item's colour tag or its kind's default, name, figures 10px muted, path `text_disabled`. *drawn*
- [ ] **Folder rows** — twirl, folder glyph; children indented 14. *drawn*
- [ ] **Selected row** — `signal` @12% fill and a 2px `signal` edge on the left. *drawn*
- [ ] **Multi-selection** — the same on every picked row. *derived*
- [ ] **`in use` badge** — 9px, outlined in `on`. *drawn*
- [ ] **`missing` badge** — outlined in `attention`; the badge is the relink control. *drawn*
- [ ] **`proxy` badge** — outlined in `text_muted`. *spec*
- [ ] **Missing item's glyph** — the crossed-link glyph in `attention`. *derived*
- [ ] **Colour-tag square** at the row's right — shown on hover; opens the eight-colour picker. *derived*
- [ ] **Row menu** — new composition, interpret footage…, colour space ▸, set / make / use / clear proxy, relink…, move to folder ▸, move to root, rename, reveal, delete. Ordinary menu surface. *derived*
- [ ] **Drag ghost** of a row — the row's name on a `surface_3` card, pinned to the cursor. *derived*
- [ ] **Drop on a folder row** — the row takes the selection edge. *derived*
- [ ] **Drop on the Composition button** — the button takes the selection edge; opens Composition settings prefilled. *derived*
- [ ] **Empty panel** — one sentence, `text_muted`: import footage, or drop it anywhere. *derived*
- [ ] **Find-missing empty result** — "Every file is where the project expects it." *derived*
- [ ] **Bottom bar** (24) — folder · composition · import as glyph-and-word, a seam, then `proxies` (the project-wide switch; `text_primary` on). Words shed before glyphs as the panel narrows. *drawn*
- [ ] **Horizontal scroll strip** below the minimum width. *derived*

### 2.1 Interpretation dialogue

- [ ] **Interpret footage…** — the dialog pattern (§7): frame rate (use file / override, with one-click rates), alpha (ignore / straight / premultiplied + matte colour, guess), colour space, loop count, sequence frame rate, a reserved fields/pulldown row drawn disabled. *spec* (pattern); *open* (row list not yet drawn)

---

## 3. The Timeline

### 3.1 Header line (24)

- [ ] **Label** `timeline`. *drawn*
- [ ] **Composition tabs** — lowercase? **No: comp names are the user's and keep their case.** 11px; fronted `text_primary` over a 2px `signal` rule; the rest `text_muted`; hover adds a 1px `hairline_strong` outline over the tab. Dragging reorders. *drawn*
- [ ] **Tab ×** — appears on hover, `text_muted`. *derived*
- [ ] **Right-click a tab** — Composition settings…. *derived*
- [ ] **`export`** — the one filled action on the Timeline: `signal` fill, white label (grey room) / `#17110c` (graphite), 20 tall. *drawn*
- [ ] **Placeholder with no comp** — one sentence centred in the body. *derived*

### 3.2 The outline's chrome row (24)

- [ ] **Layer search well** — 20 tall, ghost text. *drawn*
- [ ] **Shy filter** and **master motion blur** — bare glyphs, `text_primary` on. *drawn*
- [ ] **⋯ menu** (layer / razor / work-area / marker / beat commands) — a bare glyph. *spec*
- [ ] **Layers / Graph mode tabs** — lowercase, the one in force over a 2px `signal` rule. *drawn*
- [ ] **Timecode and frame count** — **not on this row**; the deck's clock is the clock (§12B.5). *drawn*

### 3.3 Column headers (24) and the row (24)

- [ ] **Headers** — lowercase labels; a 1×10 `hairline_strong` seam at every boundary; dragging a group header reorders groups; dragging a seam resizes. *drawn* (labels); *derived* (seams)
- [ ] **Group 1 · switches** (6 cells, 6 apart) — visible, audio, solo, lock, shy, guide; bare glyphs, `text_primary` on, `text_muted` off; visible/audio swap glyph when off. *drawn*
- [ ] **Group 2 · identity** — twirl (8px chevron), 6px label dot, number in 10px muted mono-tabular, name. *drawn*
- [ ] **Group 3 · modes** (6 cells) — fx bypass, 3D, motion blur, adjustment, flow, collapse; blank where the kind cannot use it. *drawn*
- [ ] **Group 4 · pickers** — matte (84 → 80 here), blend (80 → 72), parent (64 → 56): 18-tall wells with the value and an 8px chevron; the matte column widens by the two mode toggles only while a matte is set. *drawn*
- [ ] **Group 5 · render time** — 10px muted figure; the header reads the whole frame's cost while measuring, `…` before the first result; the column vanishes when measuring is off. *drawn*
- [ ] **Optional columns** in / out / duration / stretch — the same 10px muted figures when shown. *derived*
- [ ] **Selected row** — `signal` @12% fill, 2px `signal` left edge. *drawn*
- [ ] **A row containing a selected property** — one step dimmer (`surface_2`). *derived*
- [ ] **Hover row** — `surface_3`. *derived*
- [ ] **Locked row** — the padlock lit; property rows read-only (wells drawn without their inset, values `text_muted`). *derived*
- [ ] **Shy-hidden rows** — gone while the filter is on. *derived*
- [ ] **Row drag (reorder)** — both halves slide, ≤100ms, no overshoot. *derived*
- [ ] **Layer group header row** — fold triangle, colour tick, name, member count, the four switches; blank elsewhere; an fx tick when the header carries effects. *derived*
- [ ] **Sound mix row** — pinned at the foot of both halves, two lanes tall, the master dB well and a twirl; not a layer. *derived*
- [ ] **Inline rename** — the name becomes a well; Escape cancels. *derived*
- [ ] **Layer row menu** — duplicate, reorder, delete, accepts lights, delete all markers, rename group / ungroup / pre-compose group on a header. *derived*

### 3.4 The fold-out (property lanes)

- [ ] **Section headings** — Retime (a single row), Transform, Masks, Paint, Effects (one per effect), Audio, plus shape / text-animator / puppet rows where the kind has them: each a 24px heading with twirl and lowercase label, selected by clicking its name. *derived*
- [ ] **Property row** (24 in the fold-out, 28 in the inspector) — stopwatch (10px square outline; filled `text_primary` when on), the ◂ ◆ ▸ navigator once animating (◆ filled while on a key), name, value well(s) spanning group 3's columns, unit rider. *drawn* (inspector rows show the anatomy)
- [ ] **Driven parameter** — a hollow ring and the word *driven* in the keyframe slot; the well shows the value and refuses gestures. *derived*
- [ ] **Separate axes** — one row per axis. *derived*
- [ ] **Mask rows** — mode, opacity, feather, expansion; mask paths selectable. *derived*
- [ ] **Paint stroke rows** — tool name, opacity, a menu. *derived*
- [ ] **Audio group** — Volume (dB) and a Waveform twirl. *derived*
- [ ] **Value drag** — the well's text goes `signal` for the gesture. *spec*
- [ ] **Scrub ladder chip** — the four rungs (×10 · ×1 · ×0.1 · ×0.01) in a `surface_3` card above the field, the one in force boxed in a 1px `hairline_strong`; summoned by the gesture, gone on release. *derived*
- [ ] **Keyed rows draw diamonds on their lanes**; **a shut layer's row draws half-scale summary marks** in `text_secondary`. *drawn* (full-scale); *derived* (summary)

### 3.5 The ruler (48)

- [ ] **Ground** — `surface_2`. *drawn*
- [ ] **The scale** — 4-frame ticks 4 tall, 1-second ticks 8 tall, labelled ticks 14 tall, all on the upper half; labels 9px muted at the top-left of each. Density adapts to zoom, never closer than 30px. *drawn*
- [ ] **Work-area handles** — 4px tabs with a 1px corner in `text_muted`, on the floor, topping out below the labels; no hover step (the resize cursor is the change); the band itself is not painted. *drawn*
- [ ] **Cache bar** (4, on the floor) — `on` for held (full / 70% / 45% by tier), steel for disk. Uncached draws nothing. *drawn*
- [ ] **Comp marker** — an 8×6 grey triangle standing on the cache bar with its 12px `surface_4` pill and 8px label. One per frame. *drawn*
- [ ] **Beat marker** — the same triangle, 1px `text_muted` stem; distinguished by a hollow head. *derived*
- [ ] **Span marker** — a hushed bar on the floor from its frame for its length; takes no gestures. *derived*
- [ ] **Layer markers** — the same flag on the layer's bar. *derived*
- [ ] **Marker label editor** — a small `surface_3` popover with one well and Apply. *derived*
- [ ] **Playhead** — 1px `signal` line with the 11×8 head; the head is the grab. *drawn*
- [ ] **Double-click ground** — above the waist makes a marker; below clears the work area. *derived*
- [ ] **Time navigator strip** — a 16px band above the ruler with the visible range as a `hairline_strong` thumb. *derived*

### 3.6 The lanes

- [ ] **Ground** — `surface_0`; outside the work area one step darker (`dim`). *drawn*
- [ ] **Lane row** (24) with a `hairline` under it. *drawn*
- [ ] **Layer bar** (16 in the row) — `surface_2` with the label colour at 30% (grey room) / 24% (graphite) over it, a 3px solid tab at its start, radius 2, **no name on the bar** unless the setting asks. *drawn*
- [ ] **Selected bar** — 1px `signal` outline. *drawn*
- [ ] **Source-reach ghost** — a 1px dashed `hairline_strong` outline spanning the whole source. *drawn*
- [ ] **Corner triangles** at a trim limit — 4px `text_muted` in the top corner. *derived*
- [ ] **Bar ends as trim handles** — resize cursor; no drawn handle. *derived*
- [ ] **Keyframe marks** (11) — diamond linear, square hold, hourglass bezier, split at the vertical centre; `text_secondary`; selected `signal`. *drawn*
- [ ] **Block box** on two or more selected keys — 1px `text_primary` box 4 inside the lanes, 3×6 end marks, the `n keys · n f` badge on `surface_4`. *derived*
- [ ] **Ease popover** — `surface_3` card: curve by name, two influence wells, stagger well and direction, Open graph, Apply. *derived*
- [ ] **Sequence layer clips** — clip boxes with the same fill rule, a 1px `hairline` between clips, source name and speed readout in 10px. *derived*
- [ ] **Sequence view (grown row)** — start/end thumbnails per clip, the speed-envelope strip beneath. *derived*
- [ ] **Overrun hatching** — `attention` 45° hatch at 4px pitch over a 12% wash, the `hold` tag, the 1px exhaustion tick. *spec*
- [ ] **Waveform** in a lane — `wave` steel-cyan envelope at 80% with the RMS core solid; multiwave ranked by value. *drawn*
- [ ] **Layer marker flags** on bars. *derived*
- [ ] **Razor line** and **snap hairline** — 1px `text_primary`. *derived*
- [ ] **Marquee** on empty ground — 1px dashed `signal`. *derived*
- [ ] **Drag preview** of bars and keys — the thing itself moving; no ghost, no lag. *derived*
- [ ] **Drop caret** for a Project drag — 2px `signal` line at the slot. *derived*
- [ ] **Comp-with-no-layers hint** — one sentence in `text_muted`. *derived*

### 3.7 The lane foot (24)

- [ ] **The dial** — 20px `surface_1` disc, 1px `hairline_strong` rim, a 12-mark tick ring outside it, a 2px `signal` index; its reading (`1 tick = 4 f`) beside it. *drawn*
- [ ] **Timeline magnet** — a bare glyph after the dial. *spec* (drawn on the earlier control mockup; add here)
- [ ] **Horizontal scrollbar** — 4px track on `surface_0`, `hairline_strong` thumb. *drawn*

### 3.8 The outline's foot (24)

- [ ] **Ease strip** — the three interpolation marks and `ease` (opens the Easing panel). *drawn*
- [ ] **reverse · copy · paste at playhead**. *drawn*
- [ ] **Group toggles** — switches · modes · pickers, `text_primary` while drawn. *drawn*
- [ ] **Measuring clock** — `text_primary` on. *drawn*
- [ ] **Graph mode's own commands** — tangent modes (auto / clamp / free), the lens pair, Auto fit, Easing… — share this row in Graph mode. *spec*

### 3.9 The Audio timeline panel

- [ ] **Rows as tracks** — two lanes tall; outline cell with mute, solo, fx, twirl, number, name, and the Wave / Spectral chip at the right. *derived*
- [ ] **Clip box** — header strip (colour box, name, fx, add effect, twirl), gain line as a 1px `text_primary` line, fade ramps as `text_muted` curves, waveform or spectrogram beneath. *derived*
- [ ] **Crossfade** — the overlap, with both curves. *derived*
- [ ] **Fade menu** and **Custom fade box** — the five shapes; the two-curve editor on `surface_3` with the Keep level tick. *derived*
- [ ] **Dimmed picture rows** with **Detach audio**. *derived*
- [ ] **Volume rubber band** — a 1px `text_primary` line with 6px handles. *derived*

---

## 4. The Graph editor (a Timeline mode)

- [ ] **Pane** — `surface_0` paper; `hairline` minor lines; `hairline_strong` at zero and 100%; axis figures 9px muted in the fixed right gutter. *spec*
- [ ] **Curves** — the four-step ramp, 1px; a static property a flat line; bezier / linear / hold segments drawn distinctly. *spec*
- [ ] **Keys** — the shapes as on the lanes (circle for bezier here); selected `signal`. *spec*
- [ ] **Tangent handles** — `text_muted` stems, `signal` while grabbed; broken handles draw no joining line. *spec*
- [ ] **Speed lens** — one dot per key side; Vegas envelope when on. *derived*
- [ ] **Transform box** — the lanes' block box, four edge grabs, the readout pill under it. *derived*
- [ ] **Key readout row** at the outline's foot — frame, value + unit, two influence wells. *spec*
- [ ] **Double-click a key** — frame / value / in / out wells in a small popover. *derived*
- [ ] **Keyframe speed… window** — the dialog pattern, four wells and a Continuous tick. *derived*
- [ ] **Beat markers as vertical lines** — 1px `text_muted`. *derived*
- [ ] **Waveform ghost** behind curves — `wave` at 30%. *derived*
- [ ] **Independent outline scroll** — a scrollbar at the outline's right. *derived*

### 4.1 The Easing panel

- [ ] **The unit box** — `surface_0` square with the curve in `text_primary`, two `signal` handles on `text_muted` stems, the four numbers as wells (`out x, y · in x, y`), Apply (the panel's one filled action, greyed with nowhere to send). *spec*
- [ ] **Preset tiles** — 3–4 per row, each a hairline card with its curve and lowercase name; saved shapes after the shipped ones; right-click rename / delete. *spec*
- [ ] **Selected key's fields** under the editor. *spec*
- [ ] **Popup form** (Settings option) — the smallest layout on `surface_3` over the footer. *derived*

---

## 5. The inspector — Effect controls

- [ ] **Label** `effect controls` and the subject's name at the right. *drawn*
- [ ] **Tab per recently viewed layer** — as a tab group strip when more than one. *derived*
- [ ] **Section heading** (24) — twirl, enable tick (a 10px square, filled when on), lowercase name, `reset` at the value column, render cost `3.1 ms`, close ×. Source and Transform have no tick, no reset-as-stack, no ×. *drawn*
- [ ] **Selected effect heading** — `signal` @12% fill and 2px left edge. *drawn*
- [ ] **Heading right-click** — move up / down / to top / bottom, remove, copy effect. *derived*
- [ ] **Effect rename** — `Enter` turns the name into a well. *derived*
- [ ] **Property row** (28) — stopwatch, navigator slot, name in a 74px column, control at the fixed edge. *drawn*
- [ ] **Float well** with unit rider. *drawn*
- [ ] **Slider row** — the well plus a 2px track with a 10px round thumb; the track is a second grip on one value. *drawn*
- [ ] **Angle row** — the well plus a 16px dial with a `signal` index (a true dial: full radius). *derived*
- [ ] **Point pair** — two 50px wells with the link glyph between; one unit rider after both; crosshair pick glyph. *drawn* (pair); *derived* (crosshair)
- [ ] **Colour row** — 16px swatch with a 1px `hairline`, the dropper glyph beside it. *drawn*
- [ ] **Dropdown** — a well with the value and a chevron. *drawn*
- [ ] **Checkbox** — 10px square, filled `text_primary` when on. *drawn*
- [ ] **Matte row** — layer picker well, `invert` tick. *drawn*
- [ ] **Mix row** — well and slider; blend mode and matte channel pickers beside. *drawn* (well and slider)
- [ ] **Action row** — a button in the value column, name column empty: outlined, 20 tall, lowercase. *derived*
- [ ] **Status line under rows** — one calm 10px sentence, `text_secondary`, sampled not animated. *derived*
- [ ] **Span bar** — a 4px bar: covered span in `on`, the rest `surface_2`. *derived*
- [ ] **Curve editor** — the unit square on `surface_0`, the spline in `text_primary`, points 6px, tabs per channel as lowercase labels. *derived*
- [ ] **Levels histogram** — `text_secondary` bars on `surface_0`, three handles, output range bar beneath. *derived*
- [ ] **Solve-linked camera badge** — a lowercase state phrase on the Transform heading with **Convert to keyframes** and, once edited, a 4px `signal` dot and **Clear corrections**. *derived*
- [ ] **Driven ring** — as on the lanes. *derived*
- [ ] **Foot** — `add effect` · `save preset`; Load preset… in the presets menu. *drawn*
- [ ] **Empty panel** — one sentence. *derived*
- [ ] **Group as subject** — the header's stack, Add effect targeting the group. *derived*

### 5.1 The colour picker

- [ ] **Popover** on `surface_3`, 3px-blur shadow: R G B wells across the top (0–255 or 0–1 decimals per scale), the saturation/value square, the hue strip, was/now pair, hex well, the project-colours row, the out-of-range line, Cancel and Apply as an outlined and a filled pair. *derived*
- [ ] **Dropper** beside every swatch and the focal-point row; lit `signal` while armed. *derived*

### 5.2 Value boxes and text fields

- [ ] **The well** — inset, click to type (all selected), drag to scrub; focus ring `signal`; the editor supports real text selection. *drawn*
- [ ] **Timecode wells** — the deck's clock and any dialog timecode. *drawn*
- [ ] **A read-only value** (locked layer) — no inset, `text_muted`. *derived*

---

## 6. Effects & presets

- [ ] **Search well** at the top; a live search overrides every fold. *derived*
- [ ] **Favourites** and **Presets** groups first, then categories; every heading a lowercase label with a twirl. *derived*
- [ ] **Entry row** (24) — name, origin badge for OFX / LFX in `text_muted`. *derived*
- [ ] **Star** on hover; a lit star in `text_primary`. *derived*
- [ ] **Drag ghost** onto a layer row — the entry's name on a card; the target row takes the `signal` edge. *derived*
- [ ] **Hover description** — one line in a tooltip. *derived*
- [ ] **Empty presets folder hint**. *derived*

## 7. Scopes

- [ ] **Header picker** — waveform (luma / RGB) · vectorscope · histogram. *derived*
- [ ] **Trace and graticule** — `ScopeColours::STANDARD`, exempt from the theme; the chrome around them neutral. *derived*
- [ ] **"Computed at half" note** — 9px `text_muted`. *derived*
- [ ] **Colour workspace** — two scopes stacked in a bare column. *derived*

## 8. Audio panel and Mixer

- [ ] **Audio panel sections** — beat-marker generation (source picker, sensitivity and spacing wells, range, Generate, Clear beat markers), Selected layer (Volume and Pan with stopwatches, fade in/out wells with three curve chips, Drive with audio…, Lower behind…), Levels (stereo bars with peak hold and the clip lamp). *derived*
- [ ] **Level meters** — `on` for the bar, `attention` for the held peak, `fault` for the clip lamp — the three state marks used as meters. *derived*
- [ ] **Mixer strip** — name in the label colour, pan pot (a true dial), fader (2px track, round thumb), stereo meters, dB well, mute and solo; the Master strip with the limiter lamp. *derived*
- [ ] **Beat tapping** — nothing drawn; a marker appears. *derived*

## 9. The node graph surfaces

- [ ] **Canvas** — `surface_0` with a `hairline` dot grid that thins as it zooms. *derived*
- [ ] **Node** — `surface_1` card, radius 2, `surface_2` header strip with tick · twirl · lowercase name; selected border `signal`; bypassed dashed; the Custom shader box's header washed in `curve[0]`. *derived*
- [ ] **Sockets and wires** — the five port colours (content family); filled = wired; a dragged wire dashed. *derived*
- [ ] **Legend strip** along the canvas floor. *derived*
- [ ] **Header switches** — Auto-wire and Heal (toggles, `text_primary` on), frame-all, zoom readout. *derived*
- [ ] **Node panel** — the selected node's parameter rows, the inspector's rows. *derived*
- [ ] **Nodes workspace** — the graph with the short Timeline beneath, the small Viewer with its whole bar top right, the Node panel lower right. *derived*
- [ ] **Shader editor** — the inner graph and its Parameter box, the source as Plex Mono (the one place mono is honest). *derived*
- [ ] **Hierarchy panel** — an indented, foldable read-only tree, 24px rows. *derived*
- [ ] **Debug panel** — plain readouts, 9px, Plex Mono where it shows raw values. *derived*

---

## 10. Windows and dialogs

The pattern (Rams §12A.3, canonical §12A.4): a 32px title strip with the lowercase label and the
subject's name, an optional 26 → **28** tab row, label-left rows of 32 with the label in a fixed
column, lowercase group labels over rules, a 48px footer with a 10px summary line and at most one
filled action, buttons sized to content (12 / 16 either side), a stacking footer when they will
not fit. `Enter` presses the focused control; the default action holds focus and wears the
`signal` edge. Every window drags by its own body, remembers its place, dismisses on the scrim
and on Escape.

- [ ] **Composition settings / New composition** — name, preset, size with the aspect lock and the shape reading, frame rate with presets, duration as timecode, background swatch, motion blur (angle, samples); Cancel + Create/Save. *spec*
- [ ] **Pre-compose** — name, the attribute pair (one disabled for a multi-selection), adjust duration, open the new composition; Cancel + Pre-compose. *spec*
- [ ] **Project settings** — Anti-aliasing (with the "using instead" line), Colour (OCIO path with Choose… / Clear and the loaded line, working space). *spec*
- [ ] **Export** — one scrolling page: Output / Time / Picture / Colour / Audio / Metadata tabs as a scroll-spy; the preset strip (list, Edit, Save as…); every row per the spec; disabled sections deaf-not-faded; the footer's summary and refusal line; the queue list with per-item progress, status and cancel; the when-done ticks. *spec*
- [ ] **Export queue** (window or section) — rows: comp, range, preset, destination, status word, a 48×3 progress bar, cancel ×; failed rows keep their settings. *derived*
- [ ] **Settings window** (880×640, resizable) — a 160px sidebar of pages, a search well in the title strip, sections separated by a rule and 6px of air, rows of label · (no explanatory sentence) · control; General, Appearance, Interface, Performance, Cache, Preview, Colour, Export, Keymap, Autosave, Plugins, Language, Updates. *spec*
- [ ] **Appearance page** — **Grey room / Graphite** only; the theme shelf (Duplicate, Rename…, Delete, Import…, Export…) kept for user overrides; shape row removed; the Scopes and Viewer toggles; Viewer bars arrangement; smoothing. *spec*
- [ ] **Keymap page** — a table grouped by context, chord wells that capture, conflicts stated in a line. *derived*
- [ ] **Theme editor** — the token list as label · swatch rows; Save a copy…. *derived*
- [ ] **Theme name dialog** — one well, Cancel + Save. *derived*
- [ ] **Keyframe speed…** — four wells and the Continuous tick. *derived*
- [ ] **Layer settings** (solid / camera / light settings) — the pattern. *derived*
- [ ] **Camera settings** — the pattern; the camera's zoom / depth of field rows. *derived*
- [ ] **Time stretch** — stretch factor, new duration, hold in place: which end. *derived*
- [ ] **Number dialog** (a single value asked for) — one well. *derived*
- [ ] **Expression editor** — a Plex Mono editor well, the error line in `fault` text on `surface_2`, Cancel + Apply. *derived*
- [ ] **History** — a list of undo entries, the current one marked by the `signal` edge. *derived*
- [ ] **Recovery** — 350 wide, one sentence, three stacked full-width actions (108 footer). *spec*
- [ ] **Clear-cache confirmation** — the one control that asks; one sentence, Cancel + Clear. *spec*
- [ ] **Update dialog** — the sentence, progress bar, Restart / Not now; **Before you update** as a window of its own with the notes and Not now / Continue. *derived*
- [ ] **AE import report** — a table of what mapped and what did not, in the dialog pattern. *derived*
- [ ] **Startup failure** — the splash's window carrying the failing line in `attention` and the log. *derived*
- [ ] **First-run screen** — one window, two cards (AE-style / Vegas-style), the automatic-updates tick, Skip. *spec* (pattern only)
- [ ] **Welcome screen** — 560 column: the wordmark at 22, two 63px start cards on `surface_1` behind a hairline, the recents strip (label + `clear`), 52px rows with 64×36 thumbnails, name over path, date, ×; a 28px footer with the version and two outlined links. No filled action. *spec*
- [ ] **About box** — the mark, the wordmark, the version, licences, names; **no joke line**. *spec*
- [ ] **Splash** — ~460×300 `surface_0`, mark at 96, wordmark, version, the boot log in Plex Mono with failures in `attention`, a 2px `signal` progress hairline along the bottom. *spec*
- [ ] **Marker editor** — a small popover: label well, Apply. *derived*
- [ ] **Interpretation dialogue** — §2.1. *open*

---

## 11. Popups, menus, transient surfaces

- [ ] **Menu surface** — `surface_3`, 1px `hairline`, 3px-blur shadow at 14%, 24px rows, 11px `text_secondary`, chords at the right in 10px `text_muted`, ticked rows with the set's tick, submenu chevrons, the safe triangle; disabled rows `text_disabled` with "(Not implemented)". *spec*
- [ ] **Context menus** — the same surface everywhere (rows, bars, keys, tabs, panes, the ruler). *spec*
- [ ] **Tooltip** — `surface_3` card, 9px, one or two words, the chord after a dot; ~500ms delay. *derived*
- [ ] **Command palette** — top-anchored `surface_3` card, a 20px search well, 24px result rows with a category label at the right and the chord; the highlighted row `surface_4`. *derived*
- [ ] **FX console** — opens on the pointer: search row with the opening chord as a label and the snapshot glyph; category strip of lowercase labels; the list with name, category, a 16px preview swatch; the one-sentence foot. Minimum 320, grows to 720. *derived*
- [ ] **Scrub ladder chip** — §3.4. *derived*
- [ ] **Drop-target highlights** — 1.5px dashed `signal`. *spec*
- [ ] **Drag ghosts** — pinned, no lag, no overshoot. *spec*
- [ ] **Error banner** at the top of a panel — a `surface_2` strip with a 2px `fault` rule on its left, one sentence, one lowercase action; never modal, nothing flashes. *spec*
- [ ] **Completion notification** (export finished) — the same banner form with `on`, and *Reveal in folder*. *derived*
- [ ] **Expression error banner** — as above in `fault`; the expression disabled, the keyframed value rendered. *derived*
- [ ] **Focus ring** — 1px `signal`, 1px outside, on every focusable control; no special focus for the value field. *spec*
- [ ] **Keyboard-focus traversal order** — reading order; a modal is its own scope. *derived*
- [ ] **Reduced motion / animation level** — All ≤100ms ease-out; Minimal ~50ms; None instant. *spec*

---

## 12. Tokens, type, marks — the checklist behind every line

- [ ] **Surfaces** — five per room plus the surround, values in Rams §2. *spec*
- [ ] **Text** — four tiers, contrast figures in Rams §2.3. *spec*
- [ ] **Hairline / hairline_strong**. *spec*
- [ ] **Signal** and its label colour per room; **never sets type**. *spec*
- [ ] **on / attention / fault**. *spec*
- [ ] **Layer family** — six hues at one value and chroma; four reserved. *spec*
- [ ] **Curve ramp** — four steps. *spec*
- [ ] **Port colours** — five, for the graph. *derived* (values not yet chosen at the new lightness) — *open*
- [ ] **Cache** — held / disk. *spec*
- [ ] **Marker grey** — `#565656` / `#c4c4c4`. *spec*
- [ ] **Waveform** — one steel and the multiwave ranking. *spec*
- [ ] **Scope colours** — `STANDARD`, exempt. *spec*
- [ ] **Selection** — `signal` edge + 12% fill; **keyframe selection is `signal` too**. *spec*
- [ ] **Shadow** — 0/1/3 at 14%. *spec*
- [ ] **Type scale** — 9 label / 10 units & notes / 11 body & wells / 13 dialog emphasis / 15 the deck clock / 22+ outside chrome. Nothing bold. *spec*
- [ ] **Mono** — Plex Mono for the boot log, paths, expression source, the Debug panel only. *spec*
- [ ] **Icon set** — 16 grid, 1.25px, butt caps; the Channels mark keeps its colour. *spec*
- [ ] **Chrome labels setting** (words / icons / icons everywhere) — kept; words default. *spec*
- [ ] **Density** — Regular; Compact drops the row to 20 and the property row to 24. *spec*
- [ ] **Hit targets** — 44 chrome, 24 + 32 slop dense; the rail 44×44. *spec*
- [ ] **Voice** — sentence case throughout, lowercase labels, British English, no exclamation marks, no joke. *spec*

---

## 13. The open decisions, gathered

1. **Footage-mode and Layer-mode strip** (§1.2) — anatomy not drawn anywhere; propose a 24px `surface_2` strip under the picture with in / out wells and the slip readout.
2. **Multi-view active-view mark** (§1.2) — proposed: a 2px `signal` rule under the active view's own strip.
3. **Compare / wipe divider** (§1.3) — proposed: 1px `text_primary` with a 12px grab.
4. **Interpretation dialogue rows** (§2.1) — the row list is the spec's; the drawing is owed.
5. **Port colours at the new lightness** (§12) — five hues to pick at the layer family's value and chroma.
6. **Tool options panel** — hung off the rail (Rams §12B.1) rather than on the top line; confirm the placement before the toolbar is reworked.
7. **The deck's Fill cache action** — a word on the deck, or only in the Preview panel and the menu.
8. **Compact under the module** (Rams §12A.2) — whether a 20px row can carry the 18px pickers within KD-2.
9. **`tnum` versus mono for timecode** (Rams §7.2) — the one experiment the type system depends on.
10. **The grey room beside a graded picture** (Rams §2) — the experiment everything depends on.
