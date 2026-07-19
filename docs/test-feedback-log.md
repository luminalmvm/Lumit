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

- [ ] **UI-1** Linked-property value boxes still clipped — specific to properties with the
  link control (anchor, position, scale). The link icon steals width.
- [x] **UI-2** Clicking an effect property's *name* in the layer area should highlight the
  layer; currently doesn't. — done: effect-row click now sets `selected_layer` in both the
  Timeline layer area and the docked effects panel.
- [x] **UI-3** Project search bar (K-… §3.1): subtree-aware name filter across the top; folders open to reveal matches.
- [x] **UI-4** (K-157) Project info box now fixed-height (no jump between selections) with a
  64×48 footage thumbnail drawn from the Viewer's decoded frame. (Brief lag showing the new
  clip while it decodes — same as the Viewer.)
- [x] **UI-5** Lane keyframe selection: Shift and Ctrl both toggle now — click gesture and the
  drag-marquee (a Shift/Ctrl box deselects covered keys instead of only adding).
- [ ] **UI-6** Layer area: selecting a property *name* (Transform, an effect, …) should
  support multi-select, so a user can key several at the same point at once.
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
