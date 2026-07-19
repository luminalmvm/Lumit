# Test-feedback log — 2026-07 pass 2

Working tracker (not a spec). Owner feedback captured verbatim-in-intent, with stable
IDs so nothing is lost and progress can be ticked off. When an item lands, tick it and
note the commit. Decision-sized items are logged in `docs/02-DECISIONS.md`; effect
changes update `docs/08` and ship their oracle test; new concepts update `GUIDE.md`.

## Reusable primitives (build first — several items depend on these)

- [ ] **P1 — Matte/depth-input combobox.** Replace the "after effects" bool with a
  combobox next to the layer picker: **None / Masks / Effects and Masks**. Applies
  anywhere a layer-input matte/depth is selected (matte key, DoF depth, …). Remove the
  separate bool box everywhere. (from FX-4)
- [ ] **P2 — Channel-colour picker.** A reusable three-colour selector (default red /
  green / blue, per Screenshot_135) used by RGB split and chromatic aberration, designed
  to be reused by more effects. (from FX-10, FX-9)
- [ ] **P3 — Edges mode enum.** Transparent / Repeat / Mirror, reusable wherever edges
  can become visible (radial blur already has it; shake, etc.). (from FX-11)
- [ ] **P4 — Collapsible "twirl" sub-section.** A disclosure group inside an effect's
  params to hide controls the user does not always need. First user: shake's z / extra
  axes. Reusable across effects. (from FX-11)
- [ ] **P5 — Value-range policy (K-decision).** Unless a property name contains `%` or a
  0–1 ratio is genuinely the natural unit (e.g. vignette roundness), prefer pixel / real
  units with `0..inf` (or wider signed) ranges rather than 0–1. Audit existing effects and
  widen where it helps. (from FX-6)

## UI

- [ ] **UI-1** Linked-property value boxes still clipped — specific to properties with the
  link control (anchor, position, scale). The link icon steals width.
- [x] **UI-2** Clicking an effect property's *name* in the layer area should highlight the
  layer; currently doesn't. — done: effect-row click now sets `selected_layer` in both the
  Timeline layer area and the docked effects panel.
- [ ] **UI-3** Project tab: search bar across the top of the whole panel (files/layers).
- [ ] **UI-4** Project panel selected-layer info box: give it fixed padding so switching
  layers doesn't shift the info placement; clicking footage shows a small thumbnail
  preview *in that box*, not in the viewfinder.
- [~] **UI-5** Lane keyframe selection: Shift and Ctrl should both allow *deselect* too
  (Ctrl works, Shift doesn't) — including within a drag-marquee. — click gesture done
  (Shift now toggles like Ctrl); the drag-marquee deselect path still to do.
- [ ] **UI-6** Layer area: selecting a property *name* (Transform, an effect, …) should
  support multi-select, so a user can key several at the same point at once.
- [ ] **UI-7** Copy/paste keyframes doesn't work — fix.
- [ ] **UI-8** Graph view scroll also scrolls the layer area, and the scrollbar sits in the
  graph view. Move the scrollbar back to the right of the layer area so both scroll
  independently. (Layer view scroll is already correct.)
- [ ] **UI-9** Dropper tool cursor should be the dropper the whole time the tool is active
  (currently only over the viewfinder). The magnifier preview appearing only over the
  viewfinder is correct.
- [ ] **UI-10** "Save stack as preset" should save only the effects/keyframes the user has
  highlighted: all non-keyframed values as set, plus exactly the selected keyframes from
  the selected effects — nothing unselected.
- [ ] **UI-11** Flow input rate: make it a textbox the user types into (not a dropdown),
  and keyframeable like any other property.
- [ ] **UI-12** Per-layer motion-blur toggle not visible — space seems reserved on the
  right of the layer area but the toggle isn't drawing.
- [x] **UI-13** Importing footage should auto-highlight it in the Project tab and switch to
  that tab if not already there. — done: import selects the new item and raises the Project
  tab (`focus_project_tab` flag consumed by the shell).
- [ ] **UI-14** Bottom timeline bar: the graph-option buttons are slightly clipped — make
  room.
- [ ] **UI-15** Viewfinder: in soft mode the zoomed preview spills over the border edges —
  it must sit behind the border. In round mode the bottom bar should be a pill spanning the
  bottom.

## Effects

- [ ] **FX-1** Posterize time doesn't work (5 fps in a 24 fps comp looked normal). Also its
  scope should be: only this layer's own effects, or the default (everything below in the
  stack).
- [x] **FX-2** Split Blur into three effects — **Gaussian** (Radius), **Directional**
  (Length, Angle), **Radial** (Amount, Centre X/Y, Type = Spin/Zoom, Edges). Directional
  length and radial amount should exceed 100. — done (K-137): three separate effects; old
  "blur" loads as Gaussian; Length/Amount hard-unbounded above; Edges kept on Radial.
- [x] **FX-3** Sharpen is an unsharp filter — rename it **Unsharp Mask**, and add a plain
  **Sharpen** (amount). — done (K-138): existing effect relabelled "Unsharp mask"; new
  "Sharpen" is a 3×3 high-pass with an Amount dial + oracle test.
- [ ] **FX-4** Matte after-effects → combobox (see **P1**); remove the bool everywhere.
- [x] **FX-5** Saturation should exceed 200. — done (K-135): hard cap lifted, slider to 400 %.
- [x] **FX-6** Vignette softness now `0..inf` (K-135), roundness stays 0–1.
- [x] **FX-7** Hue shift preserve-luminance bool (K-136): on = constant-luminance, off = plain-RGB spin.
- [x] **FX-8** Temperature widened (K-135): slider ±150 / hard ±200, per-unit gain 0.5→0.75.
- [ ] **FX-9** RGB split: add per-channel R/G/B amount sliders; give wavelength more
  samples, with an extra property so the user controls sample count.
- [ ] **FX-10** Chromatic aberration: add RGB-split's wavelength, and a way to pick the
  three colours (default r/g/b, per Screenshot_135 / s_distortchroma). Reuse **P2**.
- [ ] **FX-11** Shake: keep Amp/Freq/Rotation; add a **twirl** sub-section (**P4**) holding
  x/y/z shake amp+freq (z replaces zoom). Remove auto-scale; replace with edges
  Transparent/Repeat/Mirror (**P3**).
- [x] **FX-12** Block glitch: Seed should always be the second-last property, before Mix.
  — done: Seed moved to second-last in the schema (read by id, so resolve/GPU unaffected).
- [ ] **FX-13** Scanlines: collapse to a single Intensity/Darkness property (the two
  currently do the same thing).
- [ ] **FX-14** Datamosh: allow intensity > 1; rename it; add control over streak
  duration (imitating I-frame cadence) — currently works but is not noticeable enough.
- [ ] **FX-15** Flash feels off; blocked on audio fixes before it can be tested.
- [x] **FX-16** Glow (K-135): default threshold 0.8, knee label → Softness, radius px 0..inf.
- [ ] **FX-17** Echo: default mode Screen (needs adding), add the other blend modes, allow
  more than 8 echoes.
- [ ] **FX-18** Accumulation motion blur → rename to **Motion Blur**; add the option to
  force the flow settings on all layers at export time (without changing them in the comp);
  currently appears to do nothing — fix.
- [ ] **FX-19** Motion blur → rename **Fast Motion Blur**. Fix the blocky/cut-out
  artefacts: scale blur by confidence rather than gating, or drop confidence entirely. Add
  a debug view property: rendered output / motion vectors / per-pixel confidence.
- [ ] **FX-20** Transform: default anchor X/Y (and position) to the layer's centre, varying
  by the layer it's applied to — not 0,0.
- [ ] **FX-21** Matte effect: extra controls per Screenshot_136 (Keylight-style keyer —
  screen colour/gain/balance, despill, screen matte clip/rollback/shrink/softness/despot,
  inside/outside masks, fg/edge colour correction, source crops). Scope TBD with owner.
- [ ] **FX-DoF** (deferred by owner until the rest are sorted) — fuller DoF look.

## Additions / general bugs

- [ ] **GEN-1** Blend modes need Darken / Subtract options.
- [ ] **GEN-2** Add a **Vibrancy** effect to complement Saturation.
- [ ] **GEN-3** Lanes: layers should move either side of the composition bounds (before 0,
  past the end) without trimming. Also fixes long-import auto-trim.
- [x] **GEN-4** Audio fixed (K-141): the comp mix now re-derives from the document each
  frame, so mute, move, trim (active span) and delete all take effect live. Caveat: editing
  one of several audio layers mid-playback has a brief re-decode snap; all-silent is instant.
  Unblocks FX-15.
- [x] **GEN-5** Default the lane-timeline grid to **time**, not beats. — done.
