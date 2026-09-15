# Lantern, complete surface inventory

**What this is.** Every surface, control and state Lumit draws, as
[07-UI-SPEC.md](../07-UI-SPEC.md) and the shipping frontend list them, with how **Lantern**
draws each one. It exists to be walked top to bottom, one line at a time, so that nothing the
application does is left without a ruling before anything is built. The rulings are argued in
[15-DESIGN-LANTERN.md](15-DESIGN-LANTERN.md); this document only applies them.

It is the twin of [DESK-INVENTORY.md](DESK-INVENTORY.md), which does the same job for
Desk, and the two carry the same lines in the same order so they can be read side by side.

**How to read a line.** Every item is a checkbox with a status:

- **drawn** - it is on [mockups/lantern-shell.html](mockups/lantern-shell.html) and can be
  judged by eye.
- **spec** - the Lantern document rules on it, but the mockup does not draw it.
- **derived** - no explicit ruling; the treatment follows mechanically from the system (the
  four radii, the card title, the field, the accent list). Stated here so it can be disagreed
  with.
- **open** - a decision is needed before it can be built. Gathered again at the end.

Tick a box when the line has been read and either accepted or annotated. A line that turns out
to be wrong is corrected in the Lantern document, not here.

**The system in one paragraph.** Two rooms (Day, Night) differing only in the room colour.
Lumit's own dark ramp arranged as cards: `card` is `surface_1`, `section` `surface_2`, `field`
`surface_3`, `raised` `surface_4`, `sunk` `surface_0`, and the room is Lumit Light's canvas.
The spruce accent, spent on fills, with a wider but still closed job list. Four radii that each
mean one thing: card 16, section 12, field and button 7, lane content 3, plus the pill for
actions and active segments. Card titles are the one kicker: 10px caps, tracked, centred, with
a lit accent dot at the card's corner. Everything else is sentence case. A card casts a shadow
into the room and nothing else casts. No hairline separates panes, and inside a card a hairline
separates one section from the next and bounds the Viewer stage.

---

## 0. The application shell

### 0.1 The window

- [ ] **Window chrome** - opens maximised, remembers geometry; nothing to draw. *derived*
- [ ] **The room** - `#d9d9d6` by day, `#0b0c0e` at night; 10px between cards and 10px from the window edge. *drawn*
- [ ] **UI scale setting** (100 / 125 / 150%) - radii and gaps scale with it. *derived*

### 0.2 The top band (40 as built, 44 drawn)

- [ ] **The mark** - twin keyframes at 16, in the room, left. *drawn*
- [ ] **Menu bar, in-window** (Windows and Linux) - nine sentence-case words in `room_ink`. *drawn*
- [ ] **Menu bar, macOS** - the system bar; the band keeps the mark, the palette and, under Left, the workspace pill. *spec*
- [ ] **Workspace pill** (28) - a segmented pill, the labels the body face in sentence case, the fronted name accent-filled and inset by `pillInset` (3) on every side with its corner the pill's less 3. **Rides the toolbar row under Top and the band under Left** (§12B.1). *drawn*
- [ ] **Tool options pill under Left** - on the band beside the workspace pill; the strip above the dock is not mounted. *built*
- [ ] **Command palette pill** - at the right, with the chord as a muted hint. *drawn*
- [ ] **Update state in Help** - the row's text changes in place; no colour. *derived*

### 0.3 The toolbar

- [ ] **Position setting** - Top or Left, machine-local, the identical toolbar either way (§12B.1). This is an amendment to 07 §1.7. *drawn*
- [ ] **The tool pill** - every group from §1.7 in one capsule, in order, seams inside it, snapping at the end. *drawn*
- [ ] **A tool button** - 28px, bare inside the pill, glyph in `text_secondary`; hover `raised`. *drawn*; as built the pills are 32 on a 44 band
- [ ] **The armed tool** - accent-filled, glyph in `accent_ink`. The only filled thing in the pill. *drawn*; as built a 26 accent disc centred in a bare cell with the glyph in `surface_0`, in both positions
- [ ] **A group** - carries the member last used and a corner triangle. *drawn*
- [ ] **A group's flyout** - a `float` card with the float shadow, one 24px row per member with glyph, name and chord; unbuilt members drawn `text_disabled`. Opens below under Top, to the right under Left. *spec*
- [ ] **A disabled tool** - `text_disabled`, declines the click and the chord, tooltip says it is not built. *derived*
- [ ] **The tool options pill** - the armed tool's own settings, beside the tool pill. Drawn deaf and labelled when the armed tool has no options, so the row keeps its width. *drawn*
- [ ] **Tool options content** - fill swatch, size, hardness, opacity, stroke swatch, stroke width, density, expansion, roto size, per §1.7's table. *spec*
- [ ] **Snapping switch** - inside the tool pill after a seam; accent-filled on. *drawn*
- [ ] **Hit targets** - a 44 band carrying 32 pills under Top; 36 in a 44 column under Left. No KD-2 concession. *spec*
- [ ] **Tooltips** - name and chord, one line. *derived*

### 0.4 The dock

- [ ] **Cards** - every pane a card at radius 16 with the card shadow, 10px apart. *drawn*
- [ ] **Card title line** (36) - the accent dot at the corner, the title centred in 10px caps tracked, the pane's own controls at the right. *drawn*
- [ ] **A tab group** - the title line carries pill tabs, the fronted one `section`-filled, the fill inset by `pillInset` (3) on every side with its corner the stadium less 3. *built*
- [ ] **Panel menu** (undock, close, maximise, help) - a circle button on the title line. *derived*
- [ ] **Drop zones during a panel drag** - five zones, `accent_soft` fill with a 1.5px dashed accent ring, the pending layout previewed as a card outline. *spec*
- [ ] **Panel drag ghost** - the card's title line alone, with the float shadow, pinned to the cursor. *spec*
- [ ] **Floating window** - a real OS window; the card fills it with no room around it. *derived*
- [ ] **Maximise a panel** (backtick) - the card fills the dock. *derived*
- [ ] **Panel minimum widths** - declared once, enforced by the seam and the slide, and **re-measured against the 10px gaps**, which a tiled dock does not spend. *open*
- [ ] **Width degradation ladder** - unchanged: ellipsise, wrap, hide optional columns, overflow, scroll. *spec*

### 0.5 The readout pills (32)

- [ ] **Order** - Lumit's own: saved, cache meters, measuring clock, notice, background jobs. *drawn*
- [ ] **Each item its own bubble**, on the room rather than in a strip. The one thing this style changes about the status line. *drawn*
- [ ] **Saved pill** - a glyph and the word; says *Unsaved* with the dot when the document is dirty. *drawn*
- [ ] **Cache meters pill** - vram, ram, disk, each a label, a 36x4 rounded bar and the exact figure. *drawn*
- [ ] **Measuring clock pill** - the clock glyph and the frame cost; `text_primary` while measuring, muted when off. It is the switch, not just a readout. *drawn*
- [ ] **Notice pill** - the latest notice with its close mark; grows to fill the row. *drawn*
- [ ] **Job pill** - one per running job (export, make proxy): the name, a progress bar, the time left and a cancel mark. Absent when nothing is running. *drawn*
- [ ] **Engine refusals** - one calm sentence in the notice pill. *derived*

---

## 1. The Viewer

### 1.1 The bar above the stage

- [ ] **Card title** `VIEWER`, centred, with the composition's name at the right. *drawn*
- [ ] **Magnification picker** - a pill with the value and a chevron. *drawn*
- [ ] **Preview resolution picker** - a pill; its menu also carries the playback behaviour as option rows that leave the menu open. *drawn*
- [ ] **Channel mark** - the one glyph with colour of its own: three overlapping circles for RGB, one for R, G or B, near-white for alpha. Not boxed. *drawn*
- [ ] **Transparency board toggle** - a circle button with the checkerboard glyph. *drawn*
- [ ] **View menu** - one circle button; the menu carries wireframes, motion paths, mask paths, gizmo visibility, layer controls, region of interest, grid, safe areas, rulers, snap to guides, clear guides, composition background with its swatch. *drawn* (mark); *spec* (menu)
- [ ] **3D view picker** - a pill. *drawn*
- [ ] **Exposure** - the number alone in a pill, with the reset mark to its left only while it is not zero. *drawn*
- [ ] **Snapshot pair** - Take and Show as circle buttons; Show `text_disabled` until a snapshot exists. *drawn*
- [ ] **Colour pipeline picker** - a pill; its menu carries the tone map row when Settings asks for it. *drawn*
- [ ] **Degradation reading** - `1920x1080 -> 960x540` in muted mono at the right, in a fixed slot. *drawn*
- [ ] **Bar overflow** - collapses from the right into a chevron circle. *derived*
- [ ] **Nothing on the bar moves as the picture changes** - every varying part sits in a slot sized for its longest value. *derived*

### 1.2 The stage

- [ ] **Stage** - `viewer_surround` `#121212`, strictly neutral, at radius 12, **bounded by a hairline** because it is 1.02:1 against the card. *drawn*
- [ ] **Surround options** - the user's slider offers neutral greys only; the themed-surround opt-in survives, off by default. *spec*
- [ ] **Transparency board** - two neutral greys, clipped to the picture. *derived*
- [ ] **Letterbox bars** - surround grey. *derived*
- [ ] **Neutrality zone** - nothing saturated within 48px of the picture except overlays on the picture itself. **The toolbar never enters it in either position.** *drawn*
- [ ] **Selection name chip** - an accent pill top-left over the picture, label in `accent_ink`. *drawn*
- [ ] **At-effect chip** - a `float` pill top-right, reading engaged while on. *drawn*
- [ ] **Preview progress bar** - on the deck's right end, a rounded 4px bar with a slow sheen; after ~150ms only, never during playback. *spec*
- [ ] **Rulers** (18) - `section` bands with a hairline seam and muted labels on a 1/2/5 ladder. *derived*
- [ ] **Guides** - 1px accent lines with a thin grab strip. *derived*
- [ ] **Grid and safe areas** - hairline lines over the picture. *derived*
- [ ] **Region of interest outline** - 1px dashed `text_secondary`, with a small clear mark. *derived*
- [ ] **Snapshot shown** - the stored picture over the live one, no border, no badge. *derived*
- [ ] **Footage and Layer mode source-time strip** - a `section` band under the picture with in and out fields and a slip readout. *open*
- [ ] **Multi-view layouts** - each view a stage with a 10px gap; the active view marked by an accent dot on its own strip. *open*
- [ ] **View lock padlock** - a circle button on the view's strip. *derived*
- [ ] **Empty Viewer** - the welcome's two start cards centred, capped at 560. *spec*
- [ ] **"Select a composition" line** - one muted sentence, centred. *derived*

### 1.3 Layer controls and overlays

- [ ] **Wireframe** of a selected layer - 1px accent. *drawn*
- [ ] **Hover box** of an unselected layer - 1px accent at 40%. *derived*
- [ ] **Scale handles** - 8px accent circles with a white ring, at corners and edge midpoints. *drawn*
- [ ] **Rotation bar** - a 1px accent line standing off the top edge. *drawn*
- [ ] **Anchor handle** - a 10px accent ring at the centre. *drawn*
- [ ] **Marquee** - 1px dashed accent with `accent_soft` fill. *derived*
- [ ] **Snap indication** - a 1px accent hairline at what caught the drag. *derived*
- [ ] **Neutral handles option** - every overlay above in `text_secondary` instead. *derived*
- [ ] **Motion path** - 1px `text_primary` path, 2px per-frame dots, 6px keyframe boxes, muted handle stems. *derived*
- [ ] **Mask paths** - 1px accent, 5px vertices, feather handles as stems. *derived*
- [ ] **Shape in flight** - the path as drawn, 1px accent. *derived*
- [ ] **Type edit** - caret and selection in the layer's own text. *derived*
- [ ] **Paint stroke in flight** - the stroke itself; brush ring per §1.4. *derived*
- [ ] **Clone source mark** - a 12px accent crosshair. *derived*
- [ ] **Roto overlay** - subject in `success`, background in `error`, refine in accent, faded to 60%; Boundary view draws the matte edge with a halo. *derived*
- [ ] **Camera gizmo and orbit circle** - 1px accent. *derived*
- [ ] **Camera-track point cloud** - depth-cued dots in `text_primary`. *derived*
- [ ] **Point-cloud selection** - picked dots in accent; the **Create null / Create solid** row is a `float` pill pair. *derived*
- [ ] **Puppet pins** - 8px accent rings, kinds told apart by glyph. *derived*
- [ ] **Planar track display** - the quad in accent with 6px corner marks. *derived*
- [ ] **Compare and wipe divider** - 1px `text_primary` with a 12px round grab. *open*

### 1.4 The tools' pointers

- [ ] **Hardware crosshair** for aiming tools, badged down-right with the tool glyph on a halo. *derived*
- [ ] **Brush ring** at brush size in picture scale, 1px with a dark halo. *derived*
- [ ] **Drawn pointers** - Hand, Zoom, Rotation, Anchor point, vertical I-beam, all at 1.5px with round caps. *derived*
- [ ] **Razor line** - a 1px `text_primary` line across every lane. *derived*
- [ ] **Dropper magnifier** - a 9x9 grid on a `float` card at radius 12, dashed rules, a solid border round the sampled region at radius 3, and the reading strip under it. *derived*
- [ ] **Armed dropper glyph** - accent-filled while armed. *derived*

### 1.5 The deck (36 as built, 52 drawn)

- [ ] **Clock** - a `field` pill at the left, 15px mono, click to type, drag to scrub. *drawn*
- [ ] **Frame count** - `f112 / 525` beside it in muted mono. *drawn*
- [ ] **Transport** - the five marks gathered in **one `section` pill** with a 34px accent-filled play at its centre. *drawn*
- [ ] **Loop mode picker** - a pill. *drawn*
- [ ] **Preview mode picker** (cached / realtime) - a pill. *drawn*
- [ ] **Quality toggle** - a pill. *drawn*
- [ ] **Audio mute** - a circle button; muted draws the muted speaker. *drawn*
- [ ] **Cache-ready meter** - a rounded bar in `success` with its figure. *drawn*
- [ ] **Fill cache action** - a ghost pill after the meter. *open*
- [ ] **Dockable Preview panel** - the same controls as a card. *derived*

---

## 2. The Project panel

- [ ] **Card title** `PROJECT`, with the item count at the left and the missing count at the right of the title line. *drawn*
- [ ] **Specimen** - a `section` sub-card: 96x54 poster at radius 8, and a two-column readout of name, size and rate, length, codec, sound. *drawn*
- [ ] **Hover-scrub** on the poster; a play mark on a sound file; a still says `still`. *derived*
- [ ] **Search field** - a `field` pill, full width, with the **colour-swatch filter** as a round swatch in the leading slot. *drawn*
- [ ] **Column headers** - muted 9.5px sentence case: Name, Items, Size, fps, Path. *drawn*
- [ ] **Rows** (26, radius 8) - a round type avatar in the item's colour, the name, figures in muted mono, the path quieter again. *drawn*
- [ ] **Folder rows** - a twirl and a folder avatar; children indented. *drawn*
- [ ] **Selected row** - `accent_soft` fill, no border. *drawn*
- [ ] **Multi-selection** - the same fill on every picked row. *derived*
- [ ] **`in use` badge** - a `success` pill. *drawn*
- [ ] **`missing` badge** - a `warning` pill, and it is the relink control. *drawn*
- [ ] **`proxy` badge** - a muted pill. *spec*
- [ ] **Colour-tag square** - on hover at the row's right; opens the eight-colour picker. *derived*
- [ ] **Row menu** - new composition, interpret footage, colour space, the four proxy commands, relink, move to folder, move to root, rename, reveal, delete. *derived*
- [ ] **Drag ghost** - the row's name on a `float` pill, pinned to the cursor. *derived*
- [ ] **Drop on a folder row** - the row takes the `accent_soft` fill. *derived*
- [ ] **Drop on the Composition button** - the pill takes the accent fill. *derived*
- [ ] **Empty panel** - one muted sentence. *derived*
- [ ] **Find-missing empty result** - "Every file is where the project expects it." *derived*
- [ ] **Bottom line** - Folder, Composition, Import as pills, then the Proxies switch. Words shed before glyphs as the card narrows. *drawn*
- [ ] **Horizontal scroll** below the minimum width. *derived*

### 2.1 Interpretation dialogue

- [ ] **Interpret footage** - the dialog pattern (§7): frame rate, alpha, colour space, loop, sequence frame rate, a reserved fields row drawn deaf. *open*

---

## 3. The Timeline

### 3.1 Title line

- [ ] **Card title** `TIMELINE`, centred. *drawn*
- [ ] **Composition tabs** - a segmented pill on a `field` track, the fronted one `section`-filled. Comp names keep the user's case. *drawn*
- [ ] **Tab close mark** - on hover, muted. *derived*
- [ ] **Right-click a tab** - Composition settings. *derived*
- [ ] **Layers / Graph** - a segmented pill, the one in force accent-filled. *drawn*
- [ ] **Export** - the accent-filled pill at the right, the one filled action the card is allowed. *drawn*
- [ ] **Placeholder with no comp** - one centred sentence. *derived*

### 3.2 The outline's chrome row

- [ ] **Layer search** - a `field` pill. *drawn*
- [ ] **Shy filter** and **master motion blur** - circle buttons, accent-filled when on. *drawn*
- [ ] **Group toggles** - Switches, Modes, Pickers as ghost pills at the right, filled while their columns are drawn. *drawn*
- [ ] **Overflow menu** - a circle button carrying the layer, razor, work-area, marker and beat commands. *spec*
- [ ] **Timecode and frame count** - **not repeated here** as drawn; the deck's clock is the clock. As built the pair sits on the 36 header line, because the deck is not always shown. *built*

### 3.3 Column headers and the row

- [ ] **Headers** - muted 9.5px sentence case; a group header drags to reorder, a seam drags to resize. *drawn* (labels); *derived* (drags)
- [ ] **Group 1, switches** (6) - visible, audio, solo, lock, shy, guide as bare glyphs; `text_primary` on, `text_disabled` off; visible and audio swap glyph when off. *drawn*
- [ ] **Group 2, identity** - twirl, a 16px round avatar in the layer-type hue with its glyph inside, the number, the name. *drawn*
- [ ] **Group 3, modes** (6) - fx bypass, 3D, motion blur, adjustment, flow, collapse; blank where the kind cannot use it. *drawn*
- [ ] **Group 4, pickers** - matte, blend, parent as 20px `field` pills with a chevron; the matte column widens by its two toggles only while a matte is set. *drawn*
- [ ] **Group 5, render time** - muted mono; the header reports the whole frame while measuring; the column vanishes when measuring is off. *drawn*
- [ ] **Optional columns** in, out, duration, stretch - the same muted mono. *derived*
- [ ] **Row** - 26 tall at radius 8, with a 2px gap to the next. *drawn*
- [ ] **Selected row** - `accent_soft` fill. *drawn*
- [ ] **A row containing a selected property** - one step dimmer. *derived*
- [ ] **Hover row** - `section`. *derived*
- [ ] **Locked row** - the padlock lit; property rows read-only, fields drawn without their fill. *derived*
- [ ] **Row drag** - both halves slide, spring-eased, with the signature settle. *derived*
- [ ] **Layer group header row** - fold triangle, colour tick, name, member count, the four switches; an fx tick when the header carries effects. *derived*
- [ ] **Sound mix row** - pinned at the foot of both halves, two lanes tall, the master dB field and a twirl. *derived*
- [ ] **Inline rename** - the name becomes a field; Escape cancels. *derived*
- [ ] **Layer row menu** - duplicate, reorder, delete, accepts lights, delete all markers, and the group commands on a header. *derived*

### 3.4 The fold-out

- [ ] **Section headings** - Retime, Transform, Masks, Paint, Effects, Audio, and the shape, text-animator and puppet rows, each a 26px heading with a twirl and a sentence-case name. *derived*
- [ ] **Property row** - stopwatch circle, the navigator once animating, the name in a fixed column, the field, the unit rider, the reset arrow. *drawn*
- [ ] **Stopwatch** - an accent-filled circle when keyed, a muted ring when not. *drawn*
- [ ] **Driven parameter** - a hollow ring and the word *driven* in the keyframe slot; the field shows the number and refuses gestures. *derived*
- [ ] **Separate axes** - one row per axis. *derived*
- [ ] **Mask rows** - mode, opacity, feather, expansion. *derived*
- [ ] **Paint stroke rows** - tool name, opacity, a menu. *derived*
- [ ] **Audio group** - Volume in dB and a Waveform twirl. *derived*
- [ ] **Value drag** - the field's number goes `text_primary` and its edge takes the accent for the gesture. *spec*
- [ ] **Scrub ladder chip** - the four rungs on a `float` pill, the one in force filled. *derived*
- [ ] **Keyed rows draw marks on their lanes**; a shut layer draws half-scale summary marks. *drawn* (full scale); *derived* (summary)

### 3.5 The ruler (48)

- [ ] **Ground** - the lane area is a `section` sub-card at radius 12. *drawn*
- [ ] **The scale** - ticks on the upper half with second labels in muted mono; density adapts to zoom, never closer than 30px. *drawn*
- [ ] **Work area** - an `accent_soft` capsule on the second row with two round accent handles. *drawn*
- [ ] **Cache bar** - rounded runs with a 2px gap: `success` for held, `cache_disk` for disk, at full, 70% and 45% height. Uncached draws nothing. *drawn*
- [ ] **Comp marker** - an 8px grey dot on the ruler's floor with its label in a `raised` pill. One per frame. *drawn*
- [ ] **Beat marker** - the same dot, hollow. *derived*
- [ ] **Span marker** - a hushed rounded bar from its frame; takes no gestures. *derived*
- [ ] **Layer markers** - the same dot on the layer's bar. *derived*
- [ ] **Marker label editor** - a `float` popover with one field and Apply. *derived*
- [ ] **Playhead** - a 2px accent line with a 12px round accent head ringed in the lane ground. The head is the grab. *drawn*
- [ ] **Double-click** - above the waist makes a marker, below clears the work area. *derived*
- [ ] **Time navigator strip** - a band above the ruler with the visible range as a rounded thumb. *derived*

### 3.6 The lanes

- [ ] **Ground** - `sunk`; outside the work area one step darker. *drawn*
- [ ] **Lane row** - 28 pitch, no rule between lanes. *drawn*
- [ ] **Layer bar** (18 in the 28 row) - the layer-type hue at 55% over the lane ground with a solid leading edge, at radius 3. **Not a capsule**: a capsule hides where a clip starts. *built*
- [ ] **Bar label** - the bar always carries the layer's name, by request; the canonical setting governs the other shapes. *built*
- [ ] **Selected bar** - a 2px accent ring outside it. *drawn*
- [ ] **Source-reach ghost** - a 1px dashed outline spanning the whole source. *drawn*
- [ ] **Corner triangles** at a trim limit. *derived*
- [ ] **Bar ends as trim handles** - the resize cursor, no drawn handle. *derived*
- [ ] **Keyframe marks** (11) - diamond linear, square hold, hourglass bezier, split at the vertical centre, corners eased by 1px. Rest `text_secondary`, selected accent. *drawn*
- [ ] **Block box** on two or more selected keys - a `text_primary` box with end marks and the `n keys · n f` badge on a `raised` pill. *derived*
- [ ] **Ease popover** - a `float` card: curve by name, two influence fields, stagger, Open graph, Apply. *derived*
- [ ] **Sequence layer clips** - clip boxes at radius 3 with a hairline between, source name and speed readout. *derived*
- [ ] **Sequence view** - start and end thumbnails per clip, the speed-envelope strip beneath. *derived*
- [ ] **Fade wedges** - drawn as triangular ramps at a clip's ends. *drawn*
- [ ] **Overrun hatching** - `warning` hatch over a wash, with the `hold` tag and the exhaustion tick. *spec*
- [ ] **Waveform** - `waveform.rest` envelope with the rms core solid; the multiwave stack ranked by value. *drawn*
- [ ] **Razor line** and **snap hairline** - 1px `text_primary`. *derived*
- [ ] **Marquee** - 1px dashed accent. *derived*
- [ ] **Drag preview** - the thing itself moving, with the signature settle on drop. *derived*
- [ ] **Drop caret** - a 2px accent capsule at the slot. *derived*
- [ ] **Minimap strip** - the whole comp compressed under the lanes with a lighter window over the visible part. *drawn*
- [ ] **Comp-with-no-layers hint** - one muted sentence. *derived*

### 3.7 The lane foot

- [ ] **Zoom** - a track with a round thumb between the two landscape glyphs. *drawn*
- [ ] **Magnet** - a circle button, accent-filled on. *drawn*
- [ ] **Horizontal scrollbar** - a rounded thumb on a `sunk` track. *drawn*

### 3.8 The outline's foot

- [ ] **Ease strip** - the three interpolation marks and an Ease pill. *drawn*
- [ ] **Reverse, Copy, Paste at playhead** - ghost pills. *drawn*
- [ ] **Measuring clock** - a circle button, lit while measuring. *drawn*
- [ ] **Graph mode's own commands** - tangent modes, the lens pair, Auto fit, Easing, sharing this row in Graph mode. *spec*

### 3.9 The Audio timeline panel

- [ ] **Rows as tracks** - two lanes tall; the outline cell carries mute, solo, fx, twirl, number, name and the Wave / Spectral chip as a segmented pill. *derived*
- [ ] **Clip box** - a header strip with the colour box, name, fx, add effect and twirl; the gain line; fade ramps; the wave or spectrogram beneath. *derived*
- [ ] **Crossfade** - the overlap with both curves. *derived*
- [ ] **Fade menu and the custom two-curve box** - on a `float` card with the Keep level tick. *derived*
- [ ] **Dimmed picture rows** with **Detach audio** as a pill. *derived*
- [ ] **Volume rubber band** - a 1px line with round handles. *derived*

---

## 4. The Graph editor

- [ ] **Pane** - `sunk` paper inside a `section` sub-card; hairline minor lines; `text_muted` at zero and 100%; axis figures in the fixed right gutter. *spec*
- [ ] **Curves** - the canonical `curve[0..3]` ramp at 1px; a static property a flat line; bezier, linear and hold segments drawn distinctly. *spec*
- [ ] **Keys** - as on the lanes, circle for bezier here; selected accent. *spec*
- [ ] **Tangent handles** - muted stems with round dot ends, accent while grabbed. *spec*
- [ ] **Speed lens** - one dot per key side; the envelope when on. *derived*
- [ ] **Transform box** - the lanes' block box with four edge grabs and a readout pill. *derived*
- [ ] **Key readout row** - frame, value and unit, two influence fields. *spec*
- [ ] **Double-click a key** - frame, value, in and out as fields in a `float` popover. *derived*
- [ ] **Keyframe speed window** - the dialog pattern, four fields and a Continuous tick. *derived*
- [ ] **Beat markers as vertical lines** - 1px muted. *derived*
- [ ] **Waveform ghost** behind curves at 30%. *derived*
- [ ] **Independent outline scroll** - a rounded thumb at the outline's right. *derived*

### 4.1 The Easing panel

- [ ] **The unit box** - a `sunk` square with the curve in `text_primary`, two accent handles, the four numbers as fields, Apply as the accent pill, greyed with nowhere to send. *spec*
- [ ] **Preset tiles** - 3 or 4 per row, each a `section` card at radius 12 with its curve and name; saved shapes after the shipped ones; right-click rename and delete. *spec*
- [ ] **Selected key's fields** under the editor. *spec*
- [ ] **Popup form** - the smallest layout on a `float` card. *derived*

---

## 5. The inspector

- [ ] **Card title** `EFFECT CONTROLS` with the subject's name at the right. *drawn*
- [ ] **Tab per recently viewed layer** - pill tabs on the title line when more than one. *derived*
- [ ] **Section** - a `section` sub-card at radius 12 per effect, plus Transform and Source. *drawn*
- [ ] **Section heading** - twirl, an accent **toggle switch** for enable, the name, the render cost, the close mark. Source and Transform have no switch and no close. *drawn*
- [ ] **Selected section** - an accent inset ring. *drawn*
- [ ] **Heading right-click** - move up, down, to top, to bottom, remove, copy effect. *derived*
- [ ] **Effect rename** - Enter turns the name into a field. *derived*
- [ ] **Property row** (28) - stopwatch, navigator, name in a fixed column, control at the fixed edge, unit, reset arrow. *drawn*
- [ ] **Reset arrow per row** - muted, always drawn, writes the declared default as one op. *drawn*
- [ ] **Float field** with unit rider. *drawn*
- [ ] **Slider row** - the field plus a track with a round thumb. **Subject to the Range sliders setting** (§12B.6). *drawn*
- [ ] **Angle row** - the field plus a dial with an accent index. *derived*
- [ ] **Point pair** - two fields with the link glyph between, one unit rider after both, the crosshair pick. *drawn*
- [ ] **Colour row** - a round swatch with the dropper beside it. *drawn*
- [ ] **Dropdown** - a `field` pill with the value and a chevron. *drawn*
- [ ] **Checkbox** - a small square, accent-filled on. *derived*
- [ ] **Matte row** - the layer picker and the invert tick. *spec*
- [ ] **Mix row** - the field and its track, with the blend and matte-channel pickers beside. *drawn*
- [ ] **Icon-row parameter** - a run of small glyph buttons with the active one filled, used for Order, Direction and the easing presets. *drawn*
- [ ] **Action row** - a ghost pill in the value column with the name column empty. *derived*
- [ ] **Status line under rows** - one calm muted sentence, sampled not animated. *derived*
- [ ] **Span bar** - a rounded bar: the covered span in accent, the rest in `field`. *derived*
- [ ] **Curve editor** - the unit square on `sunk` with the spline in `text_primary`, round points, channel tabs as pill segments. *derived*
- [ ] **Levels histogram** - `text_secondary` bars on `sunk`, three handles, the output range beneath. *derived*
- [ ] **Solve-linked camera badge** - a muted pill on the Transform heading with **Convert to keyframes**, and an accent dot with **Clear corrections** once edited. *derived*
- [ ] **Foot** - Add effect as the accent pill, Save preset as a ghost one. *drawn*
- [ ] **Empty panel** - one muted sentence. *derived*
- [ ] **Group as subject** - the header's stack, Add effect targeting the group. *derived*

### 5.1 The colour picker

- [ ] **Popover** on a `float` card at radius 12: R, G and B fields across the top, the saturation and value square, the hue strip, the was and now pair, the hex field, the project-colours row, the out-of-range line, Cancel as a ghost pill and Apply as the accent one. *derived*
- [ ] **Dropper** beside every swatch and the focal-point row; accent-filled while armed. *derived*

### 5.2 Value boxes and text fields

- [ ] **The field** - `field`-filled at radius 7, click to type with the whole value selected, drag to scrub, a 1.5px accent focus ring outside it. *drawn*
- [ ] **Timecode fields** - the deck's clock and any dialog timecode. *drawn*
- [ ] **A read-only value** - no fill, muted. *derived*

---

## 6. Effects and presets

- [ ] **Search field** at the top; a live search overrides every fold. *derived*
- [ ] **Favourites and Presets groups first**, then categories, each heading a sentence-case name with a twirl. *derived*
- [ ] **Entry row** (26, radius 8) - the name and an origin badge for OFX and LFX as a muted pill. *derived*
- [ ] **Star** on hover; lit when set. *derived*
- [ ] **Drag ghost** onto a layer row - the entry's name on a `float` pill; the target row takes the `accent_soft` fill. *derived*
- [ ] **Hover description** - one line in a tooltip. *derived*
- [ ] **Empty presets folder hint**. *derived*

## 7. Scopes

- [ ] **Header picker** - waveform, vectorscope, histogram as a segmented pill. *derived*
- [ ] **Trace and graticule** - `ScopeColours::STANDARD`, exempt from the theme; the chrome around them neutral. *derived*
- [ ] **"Computed at half" note** - muted. *derived*
- [ ] **Colour workspace** - two scope cards stacked. *derived*

## 8. Audio panel and Mixer

- [ ] **Audio panel sections** - beat-marker generation, Selected layer, Levels, each a `section` sub-card. *derived*
- [ ] **Level meters** - `success` for the bar, `warning` for the held peak, `error` for the clip lamp. *derived*
- [ ] **Mixer strip** - the name in the layer's hue, a pan dial, a fader with a round thumb, stereo meters, the dB field, mute and solo as circle buttons; the Master strip with its limiter lamp. *derived*
- [ ] **Beat tapping** - nothing drawn; a marker appears. *derived*

## 9. The node graph surfaces

- [ ] **Canvas** - `sunk` with a hairline dot grid that thins as it zooms. *derived*
- [ ] **Node** - a `card` at radius 12 with a `section` header strip carrying the tick, twirl and name; selected border accent; bypassed dashed; the Custom shader box washed in `curve[0]`. *derived*
- [ ] **Sockets and wires** - the five port colours; filled means wired; a dragged wire dashed. *derived*
- [ ] **Legend strip** along the canvas floor. *derived*
- [ ] **Header switches** - Auto-wire and Heal as toggle switches, frame-all and the zoom readout beside them. *derived*
- [ ] **Node panel** - the selected node's rows, the inspector's rows. *derived*
- [ ] **Nodes workspace** - the graph card with the short Timeline beneath, the small Viewer card upper right, the Node card lower right. *derived*
- [ ] **Shader editor** - the inner graph and its Parameter box; the source in mono. *derived*
- [ ] **Hierarchy panel** - an indented read-only tree, 26px rows. *derived*
- [ ] **Debug panel** - plain readouts in mono. *derived*

---

## 10. Windows and dialogs

The pattern: a **card** floating over a dimmed room at radius 16 with the float shadow; a title
line carrying the dot and the caps title; label-left rows with the label in a fixed column;
sentence-case group names over hairlines; a foot with a summary line and at most one
accent-filled action; ghost pills for everything else; a stacking foot when the actions will
not fit one line. Enter presses the focused control, the default action holds focus and wears
the accent ring, every window drags by its body, remembers its place, and dismisses on the
scrim and on Escape.

- [ ] **Composition settings and New composition** - name, preset, size with the aspect lock, frame rate with presets, duration as timecode, background swatch, motion blur. *spec*
- [ ] **Pre-compose** - name, the attribute pair, adjust duration, open the new composition. *spec*
- [ ] **Project settings** - anti-aliasing with its "using instead" line, colour with the configuration path and working space. *spec*
- [ ] **Export** - one scrolling page, the six tabs as a scroll-spy pill row, the preset strip, every row per the spec, disabled sections deaf and legible, the foot's summary and refusal line, the queue with per-item progress and cancel, the when-done ticks. *spec*
- [ ] **Export queue** - rows: comp, range, preset, destination, a status pill, a rounded progress bar, a cancel mark. *derived*
- [ ] **Settings window** (880x640, resizable) - a sidebar of pages, a search field in the title line, sections as sub-cards, rows of label and control. *spec*
- [ ] **Appearance page** - the style picker (Studio, Desk, Lantern), the room picker per style, the eight-swatch scheme preview, the theme shelf, the Scopes and Viewer toggles, Viewer bars. *open*
- [ ] **Interface page** - UI scale, tooltips, **Toolbar position**, **Range sliders**, and the existing rows. *spec*
- [ ] **Keymap page** - a table grouped by context, chord fields that capture, conflicts stated in a line. *derived*
- [ ] **Theme editor** - the token list as label and swatch rows; Save a copy. *derived*
- [ ] **Keyframe speed, Layer settings, Camera settings, Time stretch, Number dialog** - the pattern. *derived*
- [ ] **Expression editor** - a mono editor field, the error line in `error` on a `section` band. *derived*
- [ ] **History** - a list of undo entries, the current one `accent_soft`. *derived*
- [ ] **Recovery** - 350 wide, one sentence, three stacked full-width actions. *spec*
- [ ] **Clear-cache confirmation** - the one control that asks. *spec*
- [ ] **Update dialog** and **Before you update** - the sentence, a rounded progress bar, the two actions. *derived*
- [ ] **AE import report** - a table in the dialog pattern. *derived*
- [ ] **Startup failure** - the splash card carrying the failing line in `warning` and the log. *derived*
- [ ] **First-run screen** - one card, two choices, the updates tick, Skip. *spec*
- [ ] **Welcome screen** - the 560 column: the wordmark, two start cards, the recents list with 64x36 thumbnails, the footer with the version and two ghost pills. No filled action. *spec*
- [ ] **About box** - the mark, the wordmark, the version, licences, names, and the one rationed joke. *spec*
- [ ] **Splash** - a card in the room: the mark at 96, the wordmark, the version, the boot log in mono with failures in `warning`, a rounded accent progress bar along its foot. *spec*
- [ ] **Marker editor** - a small `float` popover. *derived*

---

## 11. Popups, menus, transient surfaces

- [ ] **Menu surface** - a `float` card at radius 12 with the float shadow, 26px rows, sentence case, chords muted at the right, ticked rows with the set's tick, the safe triangle, disabled rows reading "(Not implemented)". *spec*
- [ ] **Context menus** - the same surface everywhere. *spec*
- [ ] **Tooltip** - a `float` pill, one or two words, the chord after a dot, ~500ms delay. *derived*
- [ ] **Command palette** - a top-anchored `float` card with a search field and 26px result rows, the highlighted row `accent_soft`. *derived*
- [ ] **FX console** - opens on the pointer: the search row with the opening chord and the snapshot mark, the category strip as pills, the list with a preview swatch, the one-sentence foot. *derived*
- [ ] **Scrub ladder chip** - §3.4. *derived*
- [ ] **Drop-target highlights** - `accent_soft` with a dashed accent ring. *spec*
- [ ] **Drag ghosts** - with the signature lag and settle. *spec*
- [ ] **Error banner** - a `section` strip inside the card with a 2px `error` edge at its left, one sentence, one ghost pill; never modal. *spec*
- [ ] **Completion notification** - the same form in `success` with *Reveal in folder*. *derived*
- [ ] **Expression error banner** - as above; the expression disabled, the keyframed value rendered. *derived*
- [ ] **Focus ring** - a 1.5px accent ring outside the control's own shape. *spec*
- [ ] **Tab traversal** - reading order; a modal is its own scope. *derived*
- [ ] **Animation level** - All at the canonical budget with the signature settle, Minimal a fast snap, None instant. *spec*

---

## 12. Tokens, type, marks

- [ ] **Grounds** - room, room_2, card, section, field, raised, sunk, float, plus the surround. Only the room is new. *spec*
- [ ] **Text** - the four canonical tiers, unchanged, with figures in §2.3. *spec*
- [ ] **Hairline** - one weight, used between sections and round the stage. *spec*
- [ ] **Accent** - spruce, fills only, the closed list in §3.1, label `text_primary`. *spec*
- [ ] **success / warning / error** - the canonical three, unchanged. *spec*
- [ ] **Layer family** - the canonical six, unchanged, plus the reserved four. *spec*
- [ ] **Curve ramp** - the canonical four, unchanged. *spec*
- [ ] **Port colours** - the canonical five. *spec*
- [ ] **Cache** - `success` and `cache_disk`. *spec*
- [ ] **Marker grey** - unchanged; the flag is a dot rather than a triangle. *spec*
- [ ] **Waveform** - `waveform.rest` and the multiwave ranking, unchanged. *spec*
- [ ] **Scope colours** - `STANDARD`, exempt. *spec*
- [ ] **Selection** - `accent_soft` fill on a row, an accent ring on a bar or a key, an inset ring on a section. *spec*
- [ ] **Shadows** - the card shadow and the float shadow, nothing else. *spec*
- [ ] **Radii** - 16 card, 12 section, 7 field and button, 3 lane content, pill for actions. *spec*
- [ ] **Type scale** - 9 mono, 9.5 muted labels, 10 caps card title, 10.5 pills, 11 body and fields, 15 the clock, 24+ outside chrome. *spec*
- [ ] **Faces** - one sentence-case sans for words and one mono for every number. *open*
- [ ] **Icon set** - the canonical 16 grid at 1.5px with round caps; the Channels mark keeps its colour. *spec*
- [ ] **Chrome labels setting** - kept, words the default. *spec*
- [ ] **Density** - Regular; Compact loses the row gap and a pixel. *spec*
- [ ] **Hit targets** - 44 chrome, 24 visual plus 32 slop dense, no concession. *spec*
- [ ] **Voice** - sentence case except the card title, British English, no exclamation marks, the one rationed joke kept. *spec*

---

## 13. The open decisions, gathered

1. **Does a card arrangement cost too much screen?** Ten pixels of gap plus the window inset is
   roughly 40 of width and 30 of height a tiled dock keeps. Measure against the dock's declared
   minimum widths before building.
2. **Appearance page shape** - the style picker and the room picker are two axes where there
   used to be one. Needs drawing before Settings is touched.
3. **The typefaces.** The palette is Lumit's own again; the faces are not. One sentence-case
   sans and one mono is the rule, and whether those stay the canonical pair is unsettled.
4. **Footage and Layer mode source-time strip** - anatomy not drawn anywhere.
5. **Multi-view active-view mark** - proposed as an accent dot on the view's strip.
6. **Compare and wipe divider** - proposed as 1px with a round grab.
7. **Interpretation dialogue rows** - the list is the spec's, the drawing is owed.
8. **The deck's Fill cache action** - on the deck, or only in the Preview panel and the menu.
9. **Which of Top or Left ships as Lantern's default**, and whether the default differs per
   style or is one setting for the application.
10. **Whether Range sliders default on or off**, which §12B.6 deliberately leaves open.
11. **The light room by day beside a graded picture** - the experiment every style is waiting
    on, and the one that decides whether Day or Night is the default room.
