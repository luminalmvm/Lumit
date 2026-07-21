# 05 — Parity checklist (living)

The tick-list for the one-for-one port. Updated in the same commit as the work,
newest state wins. ☐ to do · ◐ partial (remainder named) · ☑ done (with tests
where the row is logic).

## Phase F0 — scaffold and chrome

- ☑ Flutter project scaffolded (`flutter_ui/`, package `lumit_flutter`), analyzer clean
- ☑ Theme port: all 7 colour schemes digit-for-digit, ShapeTokens SHARP/ROUND,
  `with_accent` hover shift by mode, `label_colour`, `document_colour`,
  AnimationLevel durations — unit-tested against the Rust values
- ☑ Inter Medium bundled; type scale 16/12/12/11/12
- ☑ Icon set: 44 variants mapped to Iconoir, motion-blur mark as a CustomPainter
- ☑ Settings model: Performance / Autosave / Interface / Export defaults matching
  the engine constants — unit-tested
- ☑ Workspace persistence to JSON (schemes, shape, accent, animation, settings,
  dock layout)
- ☑ Dock: tree model (splits/tabs/panes) + default workspace byte-matching
  `default_layout()` shares — unit-tested; resizable dividers; tab pills with
  the three-state styling; solo panes bare
- ☑ Dock: drag a tab / bare-pane grip to re-dock — model + interaction, unit-
  and widget-tested (movePanel/simplify with the every-panel-once invariant; a
  ghost pill, drop-zone previews and commit-on-release over both shapes)
- ☑ Bare-pane affordances: right-click "Pop out into its own window" (surfaces
  the multi-window notice for now); the corner drag grip built (2×3 dot grid,
  drags the pane exactly like a tab)
- ☐ Pop out a panel into its own OS window (multi-window; deferred, see 04)
- ☑ Menu bar: File / Edit / Composition / Window with the full shipped item set
  (engine-backed items dispatch to the stub state and surface a notice)
- ☑ Status line: notices, error tint rule, export-progress slot
- ☑ Settings window: all five pages, every control, fixed geometry, opens on
  Appearance; scheme/shape/accent/motion apply live
- ☑ Accent colour picker (HSV square + hue strip + hex), reusable for the
  future editor-colour submenu
- ☑ Command palette: modal, fuzzy filter, keyboard selection, the shipped
  command list (incl. the hidden export alias)
- ☑ Shortcut routing: the §5 inventory table wired to the stub state, with the
  text-field focus gate
- ☑ Panel stubs: all seven panels with real chrome (Viewer surround, scope
  graticule placeholder, timeline strip skeleton) so the workspace reads right
- ☑ Splash boot card (K-008: centred card, boot lines, click to skip; the
  engine's real boot log streams in at F1) — widget-tested
- ☑ Active-panel accent edge (last click wins, one pane at a time) —
  widget-tested
- ☐ Sharp/Round: Round cards ☑ (fill, radius, padding, gap, shadow); window
  inset ☑; resize-gap hover/drag tinting ☑

## Phase F1 — bridge (in progress)

- ◐ `lumit-bridge` crate ☑ (bridge v0: hand-rolled JSON over a C ABI, loaded
  over `dart:ffi`; catch_unwind on every export, Rust-owned strings freed via
  `lumit_bridge_free_string`, single-client `Mutex` state; in-crate tests).
  flutter_rust_bridge codegen ☐ — deferred until the API surface stabilises
  (bridge v0 is hand-rolled JSON over C ABI, see 03-ARCHITECTURE §Bridge v0)
- ☑ Project open/save/snapshot/ops. Live: new project, open (`.lum`), save (to
  the loaded path, or a save dialogue when the project has no path yet — the
  egui Save behaviour, no separate Save As), document snapshot (item tree +
  can_undo/can_redo + path), new composition through the real op/undo path,
  undo/redo
- ☑ File dialogues (the `file_selector` plugin): Open project (`.lum` filter),
  Save (falls through to a save dialogue with `untitled.lum` when unsaved),
  Import footage (multi-select, filter mirrors the egui import list). Dialogue
  calls route through injectable seams so widget tests stub paths without
  plugin channels
- ☑ Import footage → `lumit_bridge_import_footage`: one footage item per file
  through the real AddItem/undo path (no auto-folder, mirroring the egui
  frontend); one calm notice for N imported, failures in the error tint. No
  media probing yet (probing/thumbnails are F2)
- ☑ Reopen the last project on launch: the workspace persists `lastProjectPath`
  (set on open/save-with-path), and a live bridge reopens it when the file is
  still on disk — a missing or unreadable file degrades to a calm status-line
  notice, never a crash
- ◐ Project panel live: item tree + type icons (footage/folder/composition/
  solid) with layer-colour tints and nesting, empty-document hint, hover fill.
  Not yet: thumbnails, relink, missing badge, selection/drag
- ☐ Session restore *within* a project (open comps, playhead, selection) — the
  per-project session the egui shell restores on open; F1 restores only which
  project file reopens, not its saved session

## Phase F2 — Viewer (not started)

- ☐ Shared-texture path (D3D11 interop) + `Texture` widget
- ☐ CPU RGBA fallback
- ☐ Transport + resolution picker + realtime tier readout
- ☐ Missing-footage slate (generated colour bars + item path)
- ☐ Eyedropper magnifier; transform overlays

## Phase F3 — Timeline (not started)

Comp tabs · ruler/markers/beats · work area · rows/columns/switches · clip bars
(trim/move/razor/overrun hatch) · outline twirls · Audio group (Volume +
waveform lane) · keyframe lanes (glyphs, drag, copy/paste) · graph lens ·
bottom bar (zoom/magnet/grid) · top row (time, search, MB master)

## Phase F4 — editors (not started)

Effect controls rows · keyframe navigators · channel picker · Effects & presets
(.lumfx) · Scopes (waveform/vectorscope/histogram) · Hierarchy · Export
dialogue + queue · Comp settings · Add mask · Recovery modal

## Post-parity fixes (owner's known rough edges — do NOT fix during the port)

Collected here as they come up so parity stays honest. Behavioural changes wait
until parity; the pure *defects* in this list (marked "defect") are different —
the port should simply not have them, and their absence must not be read as a
deviation when the two frontends are compared side by side.

Recorded 2026-07-21, from the owner:

1. **Settings dialogue z-order (defect).** In the egui Settings window (and
   similar surfaces), text can get stuck rendering behind input boxes. The
   Flutter port must keep labels and controls in one layout flow so nothing
   can overlap; do not reproduce.
2. **Timeline ruler height (design change, post-parity or at F3 build time).**
   The time ruler above the lane area should span the full two-row band —
   the egui frontend draws it one row tall even though the mouse-interaction
   region already covers both rows. Decide at F3: build the Flutter ruler
   two rows tall from the start (the owner's stated intent) and note the
   deliberate deviation here when done.
3. **Layer-area overcrowding (design change, F3).** The timeline's layer
   outline handles small panel sizes badly — switches, buttons and labels
   start overlapping as the column shrinks. The Flutter layer area needs a
   real degradation order (truncate/hide columns before anything overlaps).
   Design it during F3 rather than copying the egui behaviour.
4. **Value boxes clip their row (defect).** DragValue-style value boxes
   across most of the egui UI clip at the bottom of the row they sit on.
   The Flutter `DragValueField` must size rows to fit their controls; do
   not reproduce. Owner notes there are probably more of this kind — add
   them here as they surface.

5. **Full editor-colour customisation (design change, post-parity).** The
   accent picker should eventually live in a more advanced appearance
   submenu where every editor colour can be edited, not just the accent
   (owner, 2026-07-21; matches the long-standing theme-customisation wish).
   The single accent picker stays until then.

## Known deliberate deviations

- No migration of the eframe-persisted workspace; the Flutter frontend starts
  from defaults (03-ARCHITECTURE §Persistence).
- The Settings window opens on General, not Appearance (owner request,
  2026-07-21).
- The splash is not skippable by click (matches egui, where the boot card is
  the window; an early click-to-skip experiment was removed on owner
  feedback, 2026-07-21).
- Menus animate per AnimationLevel (egui's could not animate at all).
- macOS native menu bar deferred with the rest of the macOS pass.
