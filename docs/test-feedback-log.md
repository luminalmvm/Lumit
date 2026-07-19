# Test-feedback log — 2026-07 pass 2

Working tracker (not a spec). Owner feedback captured verbatim-in-intent, with stable
IDs so nothing is lost and progress can be ticked off. When an item lands, tick it and
note the commit. Decision-sized items are logged in `docs/02-DECISIONS.md`; effect
changes update `docs/08` and ship their oracle test; new concepts update `GUIDE.md`.

## Reusable primitives (build first — several items depend on these)

- [x] **P1 — Matte/depth-input combobox.** (done, K-142) None / Masks / Effects and masks on
  track matte + DoF depth; old bool migrated (true→Effects and masks, false→None). Owner follow-up: default is now
  **Effects and masks**; old `true`→Effects and masks, `false`→Masks (faithful, no mask loss).
- [x] **P2 — Channel-colour picker.** (done, K-143) reusable `channel_picker` widget keyed by
  `channel_colour_1/2/3` ids; chromatic aberration is the first adopter.
- [x] **P3 — Edges mode enum.** (done, K-145) Transparent / Repeat / Mirror, reusable wherever edges
  can become visible (radial blur already has it; shake, etc.). (from FX-11)
- [x] **P4 — Collapsible "twirl" sub-section.** (done, K-145) A disclosure group inside an effect's
  params to hide controls the user does not always need. First user: shake's z / extra
  axes. Reusable across effects. (from FX-11)
- [x] **P5 — Value-range policy (K-decision).** (done, K-135) Unless a property name contains `%` or a
  0–1 ratio is genuinely the natural unit (e.g. vignette roundness), prefer pixel / real
  units with `0..inf` (or wider signed) ranges rather than 0–1. Audit existing effects and
  widen where it helps. (from FX-6)

## UI

- [x] **UI-1** Linked value boxes no longer clip: pair rows tightened (spacing/padding) and the
  X/Y/Scale boxes fixed to 48 px so [X][link][Y] fits the outline column. (Eyeball.)
- [x] **UI-2** Clicking an effect property's *name* in the layer area should highlight the
  layer; currently doesn't. — done: effect-row click now sets `selected_layer` in both the
  Timeline layer area and the docked effects panel.
- [x] **UI-3** Project search bar (K-… §3.1): subtree-aware name filter across the top; folders open to reveal matches.
- [x] **UI-4** (K-157) Project info box now fixed-height (no jump between selections) with a
  64×48 footage thumbnail drawn from the Viewer's decoded frame. (Brief lag showing the new
  clip while it decodes — same as the Viewer.)
- [x] **UI-5** Lane keyframe selection: Shift and Ctrl both toggle now — click gesture and the
  drag-marquee (a Shift/Ctrl box deselects covered keys instead of only adding).
- [x] **UI-6** (K-158) Effect rows and the Retime row now use the shared list-select gesture
  (plain/Ctrl/Shift), so transform + effect + Retime names multi-select together; new **Key
  selected properties** command keys them all at the playhead in one undo step. Added
  `PropRow::Retime` (fixes the Retime "Time" row being unselectable).
- [x] **UI-7** Copy/paste keyframes fixed: egui-winit emits Copy/Paste events (not Key C/V), so the old shortcut watch never fired; now reads the events. (Nuance: needs non-empty OS clipboard, which self-heals on first copy.)
- [x] **UI-8** (K-159) Per-mode wheel router: LANE view keeps outline+lanes synced (one
  scrollbar, unchanged); GRAPH view decouples them — the layer list gets its own scrollbar at
  the outline's right edge and the graph wheel pans the curve only. (Column-resize is
  layers-view-only now, to keep the graph scrollbar draggable — eyeball.)
- [x] **UI-9** Dropper cursor now shows whenever the tool is armed (painted on a foreground
  layer at the pointer, OS cursor hidden), not just over the image; magnifier stays
  viewfinder-only. (Please eyeball the cursor across panels.)
- [x] **UI-10** (K-156) Preset save now respects the selection: nothing selected → whole stack;
  else only the selected effects (in order), with params trimmed to just their selected keys
  (params with no selected keys keep their value/animation as set).
- [x] **UI-11** (K-160) Flow input rate is now a keyframeable `Property`: a typed value field
  (0 = "Native", else "N fps") with the stopwatch + ◄ ◆ ► navigator, read at frame time so an
  animated rate keys each frame. (Lane keyframe diamonds for it not added — a bigger PropRow
  change; add/remove/navigate + value edit all work.)
- [x] **UI-12** Per-layer motion-blur toggle now drawn: it was only ever in the right-click
  menu, never the switch row. Shows as an "MB" text switch (no motion-blur glyph exists) in the
  far-right slot; flips `switches.motion_blur`.
- [x] **UI-13** Importing footage should auto-highlight it in the Project tab and switch to
  that tab if not already there. — done: import selects the new item and raises the Project
  tab (`focus_project_tab` flag consumed by the shell).
- [x] **UI-14** Bottom-bar cluster tightened (view toggle packs flush, group gaps 10→6, clip
  moved out) so the interpolation buttons are fully visible. (Eyeball in graph mode.)
- [x] **UI-15** Magnifier now paints inside a border-clipped rect with the border drawn last
  (preview sits behind it); the info bar uses the card-corner token so it's a pill under Round,
  square under Sharp. (Eyeball with a rounded theme.)

## Effects

- [x] **FX-1** Posterize time fixed: the decode planner still chose the frame-time source, so
  footage-only motion never stepped; now `posterize_sample_times` snaps the decode/sample time
  per scope (this-layer vs everything-below). Preview == export.
- [x] **FX-2** Split Blur into three effects — **Gaussian** (Radius), **Directional**
  (Length, Angle), **Radial** (Amount, Centre X/Y, Type = Spin/Zoom, Edges). Directional
  length and radial amount should exceed 100. — done (K-137): three separate effects; old
  "blur" loads as Gaussian; Length/Amount hard-unbounded above; Edges kept on Radial.
- [x] **FX-3** Sharpen is an unsharp filter — rename it **Unsharp Mask**, and add a plain
  **Sharpen** (amount). — done (K-138): existing effect relabelled "Unsharp mask"; new
  "Sharpen" is a 3×3 high-pass with an Amount dial + oracle test.
- [x] **FX-4** Matte/depth after-effects → **None / Masks / Effects and masks** combobox (K-142),
  bool removed. Real sites were the track matte + DoF depth (the "Matte key" effect has no layer input).
- [x] **FX-5** Saturation should exceed 200. — done (K-135): hard cap lifted, slider to 400 %.
- [x] **FX-6** Vignette softness now `0..inf` (K-135), roundness stays 0–1.
- [x] **FX-7** Hue shift preserve-luminance bool (K-136): on = constant-luminance, off = plain-RGB spin.
- [x] **FX-8** Temperature widened (K-135): slider ±150 / hard ±200, per-unit gain 0.5→0.75.
- [x] **FX-9** RGB split: per-channel R/G/B amount scales + a **Samples** control on the
  wavelength/spectral mode (defaults reproduce the classic split). (K-143)
- [x] **FX-10** Chromatic aberration (K-144): three tinted taps via the P2 channel picker
  (default r/g/b) + RGB-split's Wavelength/Samples spectral machinery.
- [x] **FX-11** Shake reworked (K-146): per-axis x/y/z amp+freq in a twirl sub-section (z
  replaces zoom pump); auto-scale removed, replaced with Edges (Transparent/Repeat/Mirror).
  Built the reusable **P3** `EdgesMode` enum + **P4** twirl `ParamGroup` (K-145) for other effects.
- [x] **FX-12** Block glitch: Seed should always be the second-last property, before Mix.
  — done: Seed moved to second-last in the schema (read by id, so resolve/GPU unaffected).
- [x] **FX-13** (K-147) Scanlines collapsed to a single Intensity; old darkness folded in on load.
- [x] **FX-14** (K-148) Datamosh: intensity cap lifted (>1 extrapolates), new **Streak length**
  (frames) scales the flow reach for heavier smear. No rename (per owner). NOTE: streak is
  reach-based, not a fixed I-frame-interval reset (clean frames still fall at stills/cuts); a
  strict interval was deferred. Default streak 4 makes existing instances look stronger.
- [ ] **FX-15** Flash feels off; blocked on audio fixes before it can be tested.
- [x] **FX-16** Glow (K-135): default threshold 0.8, knee label → Softness, radius px 0..inf.
- [x] **FX-17** (K-149) Echo defaults to Screen, gains the standard blend modes, cap raised
  8→16 (bounded for decode cost; higher is a later dynamic-window refinement).
- [x] **FX-18** (K-139) Renamed **Motion blur**; added **Force on all layers** (forces per-layer
  transform MB on every layer during the sample renders, comp unmutated). Note: it blurs
  transform-animated motion; footage-playback motion is held by design (adjustment-scope), which
  is why it read as "nothing" on a footage-only test.
- [x] **FX-19** (K-140) Renamed **Fast motion blur**; blocky seams fixed by scaling each streak
  by a smooth forward/backward-consistency confidence (no hard gate); added a **View** enum
  (Rendered / Motion vectors / Confidence).
- [x] **FX-20** (K-150) New layers centre their anchor on their own content (footage=natural
  size, solid=solid size, precomp/sequence/adjustment=comp) with position at comp centre. Text
  kept at 0,0 (size unknown until glyph layout; AE point-text convention).
- [x] **FX-21** (K-154) Matte key is now a Keylight-style colour-difference keyer: Screen
  colour/gain/balance, Despill (bias + amount), Screen matte (clip black/white/rollback),
  Replace method+colour, and View modes (Final / Screen matte / Status). DEFERRED (K-155): the
  spatial controls (pre-blur, shrink/grow, softness, despot), inside/outside garbage masks,
  fg/edge colour correction, and source crops — those need a multi-pass pipeline. The core keyer
  is what "properly key footage" needs.
- [ ] **FX-DoF** (deferred by owner until the rest are sorted) — fuller DoF look.

## Additions / general bugs

- [x] **GEN-1** (K-151) Subtract added at every site (Darken already existed): linear-light
  `max(dst-src,0)`, premultiplied snapshot path.
- [x] **GEN-2** (K-152) Vibrancy effect added (Colour): lifts low-saturation pixels more than
  vivid ones; one Amount dial, 0 = identity.
- [x] **GEN-3** (K-153) Layers now sit freely across comp bounds (start < 0, end > comp end);
  render/audio intersect with [0, comp_end); long imports keep full media duration. LIMIT: the
  timeline doesn't yet pan to negative time, so a pre-0 overhang draws clipped under the lane's
  left edge (flagged for a later view-model change).
- [x] **GEN-4** Audio fixed (K-141): the comp mix now re-derives from the document each
  frame, so mute, move, trim (active span) and delete all take effect live. Caveat: editing
  one of several audio layers mid-playback has a brief re-decode snap; all-silent is instant.
  Unblocks FX-15.
- [x] **GEN-5** Default the lane-timeline grid to **time**, not beats. — done.

## Later-reported bugs (this session)

- [x] **Effects & Presets scrollbar was mid-panel** (clipping names) — set the ScrollArea to
  not auto-shrink, so the scrollbar hugs the far-right edge and names render full-width.

- [x] **Scrub during playback didn't fully stop.** Clicking the timeline (or the viewer
  scrub) to move the playhead during playback halted the frame advance but left the audio
  engine running, so `is_playing` stayed true (play button stuck) and audio played on. Added
  `AppState::pause_playback` (pauses audio + clears the transport) and routed the timeline
  ruler and viewer scrub through it. Regression test added.

- [x] **Keyframe navigator parity** — one shared `◄ ◆ ►` navigator now renders identically for
  transform (incl. Position/Anchor/Scale pairs), the Retime Time/Velocity row, and effects
  (K-158); Position/Time no longer use the older glyphs.
- [x] **Retime "Time" value drag now updates the preview live** — new `retime_edit` live-edit
  state overrides the layer's retime during the drag so the viewer shows the dragged source
  frame (parity with transform/effect drags).
- [x] **Retime "Time" keyframes drag in time in the graph editor** — the domain-start (first)
  key is pinned at 0 (retime invariant) so its edit commits instead of the rebuild returning
  None and dropping the whole edit; interior/trailing keys drag freely.

---

# Pass 3 — 2026-07 (main session, one at a time)

Owner directive: work these in the MAIN session, one at a time, for correctness (the
parallel pass left several not-quite-right). No migration burden (pre-release).

## Testing notes (T)
- [x] T1 Linked value boxes clipping — pair rows keep default vertical button padding, ROW_H 20→22 for headroom.
- [ ] T2 Effect prop-name click still doesn't highlight the layer in the layer area.
- [ ] T3 Can't shift/ctrl-click to multi-select property rows in the LAYER area (regressed/missing).
- [ ] T4 Copying a keyframe doesn't work in GRAPH view (works in lane); paste works in both.
- [ ] T5 Copy/paste of a bezier key: handle LENGTH must be preserved regardless of neighbour distance, except clamped to the gap when the next/prev key is closer than the handle length (value-graph semantics).
- [x] T6 Clicking an effect title now selects the whole effect (highlights the title, joins the selection); title highlights when any of its params is selected.
- [x] T7 Shift-range now stops at section boundaries: a cross-effect (or cross-kind) shift-click just picks the target; within one effect it ranges.
- [x] T8 Flow input rate now shows a concrete fps (defaults to the comp/layer frame rate), no "Native" word; typing 0 still conforms to native.
- [x] T9 Per-layer MB looked dead because the comp MB MASTER was buried in comp settings. Added a master "MB" toggle to the timeline bottom bar; with it on, MB-switched layers blur along their transform. (T22: the comp master is the composition-level MB the owner asked for.)
- [x] T10 Graph-view toggle now uses the magnet's selectable-glyph look with a 3px right inset, so it isn't clipped. (Eyeball.)
- [~] T11 Best-effort (eyeball, please): the viewfinder now clips half the corner radius inside the border (was 1 px) so the magnified pixels stop poking into the rounded corner triangles; the bottom info bar is now a full pill under Round (radius = half its height) instead of the small card-radius rounding. If either is still off, tell me the exact artifact.
- [x] T12 Posterize now holds the carrying layer's own source (so applying it to a footage layer steps that footage); Everything-below also holds below-layers on any layer kind, not just adjustment layers. (Before, a Posterize on footage held nothing.)
- [x] T13 Effect dropdowns use a real ComboBox popup, not menu_button (two adjacent menu buttons acted like a menu bar and switched on hover). Also gives an AE-like framed combo. (Eyeball the new look.)
- [x] T14 DONE. Combined X/Y row: any effect `<base>_x`/`<base>_y` Float pair renders as ONE row (shared stopwatch keying both, shared ◄ ◆ ► over the union, two boxes) in both the Effect Controls panel and the timeline (shared `effects_rows`) — auto-applies to Radial blur Centre + the Transform effect's Anchor/Position/Scale. Plus the viewfinder pixel-picker: an eyedropper button on the row arms `EyedropperMode::Position { y_param }`, so the next Viewer click writes the clicked comp x/y to the pair (the x/y twin of the DoF Focus depth pick).
- [x] T15 Sharpen gained a Radius param (neighbour distance in px; 1 = 3×3, larger = coarser). Full 4-site + oracle.
- [x] T16 Vignette: added a Ramp param (gamma on the smoothstep falloff; default 1.0, 0.2–4, hard min 0.05). Full 4-site + oracle (non-identity ramp case).
- [x] T17 RGB split: removed the Radial option (K-161); added the shared 3-colour picker. Now a linear tinted-tap fringe — default red/green/blue tints reproduce the classic split bit-for-bit. Full 4-site + oracle.
- [x] T18 Shake: added a **Motion blur** twirl (toggle + Shutter 0–1, off by default; K-165).
  The wobble is a pure function of time, so it is sampled at 9 sub-frame placements across the
  shutter (host-side — the noise needs 64-bit ints the GPU lacks) and averaged in a dedicated
  `fx_shake_mb` kernel; translation, rotation and zoom smear together, on this effect alone.
  Off / Shutter 0 is the bit-exact single resample. Shutter measured in the shake's own phase
  (frame-rate independent), so no fps threading into the resolver. Full 4-site + CPU/GPU oracle
  (worst 1 fp16 ULP on NVIDIA).
- [x] T19 (K-164) Datamosh reimplemented as a flow-driven **streamline melt**: per pixel a walk
  follows the flow out of the -1 neighbour (re-sampling the flow each step so the smear curves),
  accumulating a melting prediction over the current frame. New params **Displacement** (frames of
  reach, supersedes Streak length), **Bloom** (accumulate ↔ reset trail) and **Reset interval**
  (seconds; a deterministic periodic I-frame ramp folded into resolve). Content-driven reset kept.
  Full 4-site + oracle (worst 1 fp16 ULP); cost cheap→moderate.
- [ ] T20 Flash still broken — likely because beat detection isn't working (beats grid shows nothing) with the audio.
- [~] T21 Echo Mode reworked: Behind + In front (effect-only orders) at the TOP with a divider (new Choice `dividers_after`), then the standard blend set; Max dropped (= Lighten); added Difference/Exclusion/Subtract/Divide. Shared `BlendMode::name`/`ALL` back the layer dropdown. NOT full 26-mode parity: HSL/burn/dodge are ill-defined on a premultiplied light trail — deliberately curated, logged as §3.13 open question (flagged at check-in). Full 4-site + oracle.
- [~] T22 The composition MB master button now lives in the timeline's top row (TL4), next to the view toggle — AE's timeline motion-blur button. (The comp-level master already exists — `comp.motion_blur` + `Op::SetCompMotionBlur`; with it on, layers whose own MB switch is set blur. The redundant accumulation-MB *effect* removal + the full DAG-forcing proposal is separate follow-up work.)
- [~] T23 Text layers now centre their anchor (rough glyph estimate) so they rotate/scale about their middle, not the 0,0 origin — previously text used `TransformGroup::default()` (anchor 0,0). Footage/solid/precomp were already centred (`centred_transform`, tested); camera is a point so 0,0 is fine. IF the layer you saw pivot top-left was footage/solid, tell me — that'd be a different bug (or an old project predating centring). FOLLOW-UP: wiring the T14 viewfinder x/y picker onto the LAYER transform Anchor/Position rows needs a transform-target variant on the eyedropper (currently effect-param only) — flagged.
- [x] T24 Added the full AE colour-blend set (K-162): 16 new modes (Colour burn/dodge, Linear burn, Darker/Lighter colour, Linear/Vivid/Pin light, Hard mix, Difference, Exclusion, Divide, Hue/Saturation/Colour/Luminosity), 26 total. Grouped dropdown with AE dividers; single source of truth (BlendMode::ALL/name) shared with the effect Mode param (T21). GPU-verified against a Rust reference. (Dissolve + stencil/silhouette/alpha operators deferred post-v1.)
- [x] T25 Audio waveform strip now maps through the shared `x_of` time axis (was a full-width stretch), so it tracks zoom, scroll and a moved audio layer's transients; off-screen peaks skipped. Made optional: right-click the strip to hide, right-click the ruler to show again (`show_audio_bar`).

## Also (A)
- [x] A1 Picker now drives BOTH modes in chromatic aberration and RGB split. Classic mode: tinted taps (K-161/T17). Wavelength mode: the physical SPECTRAL_BASIS is retired and replaced by a colour1→colour2→colour3 gradient (owner chose "replace the basis", K-163) — default red/green/blue still gives red→green→blue dispersion; other colours re-tint it. Full 4-site + oracle (custom-colour case added).
- [ ] A2 Fast motion blur: still blocky sometimes; want better sampling / a quality selector, and optionally a depth-map or motion-vector-map input to help. (3 reference shots: base, confidence, motion vectors.)
- [x] A3 Project tab: Ctrl/Cmd or Shift-click now toggles items into a multi-selection (`selected_items`); dragging any one of them into a comp brings the whole set in at once (`add_items_to_comp`). Plain click collapses back to single-select. (Shift is additive-toggle, not a range, for now — the tree order isn't threaded through the recursive rows yet.)
- [x] A4 (owner clarified: full lane parity for the Time/value lens; speed/velocity "vegas" lens deferred). The Retime Time (value) keys now render via the interactive `lane_keys` (not the read-only `draw_key_diamonds`), so they select on click, join a marquee, and drag in time like any transform/effect key. `build_lane_drag_op` gained a Retime path that slides the selected INTERIOR value keys by the drag delta, clamped strictly between their neighbours (≥1 frame) so the structural [0,dur] endpoints stay put and `from_value_keyframes` accepts it. Tests: interior key drags, endpoint doesn't. The speed lens keeps read-only diamonds for now. (Copy/paste of Retime keys still skipped — paste has no Retime target yet; graph value-lens marquee already worked via `GraphSelection::retime`.)
- [ ] A5 When a layer's last keyframe is removed, the stopwatch should switch off and the ◄ ◆ ► buttons disappear.

## Effect Controls layout (EC — ref screenshot 144)
- [x] EC1 Hairline under each property row (drawn in the shared `row_frame`, only across the outline column, so header rows stay undivided). Also gives TL1's row treatment.
- [x] EC2 The × (remove effect) is now right-aligned on the effect header (right-to-left layout).
- [~] EC3 Value boxes right-aligned for the common param types — Float, Choice, Bool now wrap their value in a `right_to_left` group (label left, value at the row's right edge). Colour/Seed/File/Layer + the X/Y pair row still left-packed (their multi-widget layouts need careful reordering) — eyeball whether the mix reads OK or those need doing too.
- [x] EC4 Reset icon (new `Icon::Reset` = Iconoir "refresh-double") left of the ×; resets every param on the effect to its schema default via `instantiate`.

## Timeline layout (TL — ref screenshot 145, AE-style columns)
- [ ] TL1 Similar row treatment to Effect Controls (dividers, aligned values).
- [ ] TL2 DEFERRED — owner (2026-07-19): "leave TL2 till after everything I've said, then we come back to that". The outline already carries most of these switches, but grouping them into the 5 AE column clusters that "move together" (AE's F4-style show/hide-a-group toggle, drag-to-reorder) is a major outline restructure. Needs decisions: which groups always show vs toggle, is it a swap (Switches↔Modes) or full drag-reorder, plus solo/lock/quality(bilinear|bicubic)/preserve-T switches that don't exist yet. Scope together with the app open, AFTER the pass-4 items are verified.
- [~] TL3 Clicking empty timeline space now reveals the comp in the Project panel (selects it as the project item); a right-click there gives "Composition settings…" and "Reveal in project" (focuses the project tab). (Kept the settings behind a right-click rather than firing a dialog on every deselect-click; comp-title click still TODO.)
- [x] TL4 Top row above the ruler (`timeline_top_row`): current time (m:ss:ff + frame) top-left, a layer-search box in the middle (filters the outline by name), and the graph-view toggle + a hide-switched-off-layers toggle (eye-closed) + MB master top-right — all moved up from the bottom bar. (Interpreted "hide-layer icon" as hide invisible/switched-off layers — eyeball whether that's the meaning you wanted.)

---

# Pass 4 — 2026-07-19 evening (owner's immediate notes from the app)

Timeline UI:
- [x] P4-T1 The top row now spans ONLY the outline column; right of it the strip is ruler
  background, the scrub click-and-drag region covers both rows (a taller grab area), and the
  playhead line reaches up through it. The time text / search / toggles pack within the
  outline width (the search box takes whatever sits between them and hides below ~36 px).
- [x] P4-T2 First use defaults the outline width to the Project (top-left) panel's measured
  width; dragging the divider overrides it and the choice is remembered per project (egui
  persisted storage keyed by the project path — survives restarts when eframe persistence is
  on; otherwise per-session). Drag seeding fixed so the first drag no longer jumps.
- [x] P4-T3 The Effects group header in a layer's twirl-down only shows when the layer HAS
  effects.
- [x] P4-T4 The Add effect / Presets toolbar row is gone from the timeline's effect dropdown
  (`RowCtx::effects_toolbar`); the Effect Controls panel keeps it.
- [x] P4-T5 Row dividers now stretch across the lane area too (layers view; in graph mode
  they stay on the outline so the curve stays clean), and run below EVERY line including the
  group headers.
- [x] P4-T6 ROW_H 22 → 24 with vertically centred content, so value boxes and the pair rows'
  link no longer sit on the divider (Screenshot_146). Applies to the timeline AND the Effect
  Controls panel (shared row frame).

General:
- [x] P4-G1 Applying an effect — from the Effect Controls toolbar, an Effects & Presets drag
  onto a timeline row or the panel, the layer right-click menu, or the command palette — now
  selects the fresh effect (all its param rows, like clicking its title) and brings the
  Effect Controls tab to the front (`focus_applied_effect` + `focus_effects_tab`).
- [x] P4-G2 Effect Controls values now sit snugly right of the panel's MIDLINE
  (`snap_to_value_column` — a shared value column), not the far right; Float, Choice, Bool,
  Seed, Colour and the X/Y pair row all use it, and the timeline's effect rows share the
  same rule. Divider clipping fixed by P4-T6.
