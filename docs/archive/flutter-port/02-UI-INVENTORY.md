# 02 — Inventory of the shipped egui frontend

Everything the egui frontend draws or handles, catalogued for the one-for-one
port. Source pointers are into `crates/lumit-ui/src/` at the branch point of
`flutter-frontend-alternative`. Spec references are the canonical sections in
docs/07-UI-SPEC.md and docs/15-DESIGN.md.

## 1. The shell

| Surface | Source | Notes for the port |
|---|---|---|
| Menu bar (in-window; File, Edit, Composition, Window) | `shell/app_update.rs` (~line 717) | Full item list in §6 below. macOS gets a native bar via muda (`native_menu.rs`) — out of scope until a macOS pass |
| Status line | `shell/app_update.rs` | Quiet notices (`app.notice`), genuine errors in the error tint (15-DESIGN §10), export progress with encoder label, autosave feedback |
| Boot splash | `splash.rs` | Small centred card listing boot log lines (`BootLine`), then the window expands into the app (K-008). On Linux/Wayland the window opens full-size with the card centred (TF-5) |
| Dock | `shell/dock.rs`, `egui_tiles` | Tiling tree of panels: linear splits (horizontal/vertical, weighted shares) + tab groups. Default workspace in §2. Simplification rules: solo panes render bare (no tab bar, K-086); single-child containers pruned |
| Tab pills | `dock.rs::tab_ui` | Rounded pill tabs: active = `surface_1` fill + accent stroke; hover = `surface_3` + `hairline_strong`; idle = `surface_2`, `text_muted`. Tab bar height 26 px. Sharp tab-bar background `surface_2`, Round `surface_0` |
| Bare-pane affordances | `dock.rs::bare_pane_ui` | Right-click anywhere → "Pop out into its own window"; 16 px drag grip (2×3 dot grid) top-right hands off to the dock's tile drag |
| Pop-out windows | `shell/panels.rs`, `Shell::floating` | A panel detached into its own OS window; hidden in the dock meanwhile; closing the window re-docks it |
| Active-panel edge | `Shell::active_panel`, `overlays.rs` | The last-clicked panel wears an accent boundary (the keyboard's home) |
| Sharp/Round pane chrome | `dock.rs::pane_ui` | Sharp: edge-to-edge, 1 px hairline gaps. Round: every pane a rounded (14 px), shadowed, padded (10 px) card; 12 px gaps and window inset; resize gap painted canvas colour, accent while dragging |

## 2. The default workspace

`dock.rs::default_layout()` — vertical root: upper band 0.68, Timeline strip 0.32
(full window width). Upper band horizontal: left tab group 0.22 (Project, Effect
controls, Effects & presets, Hierarchy — Project fronted at start-up), Viewer 0.58
(solo, bare), Scopes 0.20 (solo, bare). "Reset workspace" (Settings → General and
the Window menu) restores exactly this.

## 3. Panels

| Panel | Title | Source | What it does today |
|---|---|---|---|
| Project | "Project" | `shell/panels.rs` | Project items (footage, folders, comps, solids) with type icons and layer-type colours; footage thumbnail reusing the Viewer's decoded frame (UI-4); missing-footage badge (crossed link) + "Relink…" (TF-37); import; new-comp button; comp settings entry; measures its width for the timeline outline default |
| Viewer | "Viewer" | `shell/panels.rs`, `shell/draws.rs`, `shell/gpu.rs` | The composited frame on the exactly-neutral `viewer_surround` pasteboard; transport row; resolution picker (Full/Half/…/Auto + realtime tier readout); missing-footage colour-bars slate with item path; eyedropper magnifier overlay; transform gizmos/overlays for the selected layer |
| Timeline | "Timeline" | `shell/timeline/` | Comp tab strip (one pill per open comp, right-click menu incl. pop out); top row: current time, layer search, view + motion-blur master toggles; ruler with markers and beat markers; work area (B/N); layer rows: type glyph + colour tab, name, switches (eye, mute, solo, lock, fx, motion blur, 3D, collapse, flow), blend mode, matte, parent; clip bars with trim/move, razor, overrun hatch; property outline (twirls) incl. Transform, Audio (Volume + Waveform lane), effects; keyframe lanes with interpolation-coded glyphs, drag, copy/paste; bottom bar: zoom (1–400 %, 1.4× steps), magnet (snapping), grid mode, graph lens toggle |
| Effect controls | "Effect controls" | `shell/inspector/` | Per-layer property rows: transform rows (incl. linked pairs), effect title bars + parameter rows, per-parameter keyframe navigator (stopwatch, prev/add/next), speed/Retime rows, three-colour channel picker (K-143), eyedropper arming, reset-to-default (EC4) |
| Effects & presets | "Effects & presets" | `shell/panels.rs`, `preset.rs`, `fxops.rs` | Searchable effect list; apply to selected layer; save/load `.lumfx` effect presets |
| Scopes | "Scopes" | `shell/scopes.rs` | One scope per panel, chosen in its header: luma/RGB waveform, vectorscope, histogram; fixed `ScopeColours::STANDARD` graticule regardless of theme; holds the last frame rather than blanking (K-130) |
| Hierarchy | "Hierarchy" | `shell/hierarchy.rs` | Read-only indented tree of the active comp; precomp rows expandable; click selects the layer (switching comp if needed) |

## 4. Dialogs and floating surfaces

| Surface | Source | Notes |
|---|---|---|
| Settings window | `shell/settings.rs` | True modal, fixed 680×420+title; sidebar (150 px) of pages: General (workspace reset, autosave interval 1–60 min, copies kept 1–50, version), Appearance (colour scheme dropdown of 7, accent picker + reset, panel shape Sharp/Round, interface motion All/Minimal/None), Interface (UI scale 0.75–2.0× slider applied on release, show tooltips), Performance (RAM budget MB, disk budget, VRAM budget, clear cache, background fill, cache root folder picker), Export (default preset, filename template `{comp}`/`{preset}`/`{date}`) |
| Export dialogue | `shell/mod.rs::ExportDialogState`, `shell/export_actions.rs` | Preset stamp (incl. YouTube 1080p60/1440p60, Vertical 1080p60, Custom), codec, size, bitrate Mbps (blank = encoder default), include audio, suggested filename from template; confirming queues (one export at a time, queue drains in order) |
| Command palette | `shell/command_palette.rs` | Ctrl/Cmd+Shift+P modal; fuzzy search over app commands (save, undo, new comp, add layer, colour scheme switch, Settings, export incl. hidden "render output video mp4" alias); Enter/click runs; arrow-key selection |
| Composition settings | `shell/dialogs.rs` | Name, size, frame rate, duration for a comp |
| Add mask | `shell/dialogs.rs` | Rectangle / Ellipse / Star onto the selected layer |
| Crash recovery | `shell/dialogs.rs` | Three options: restore journal, last save, open an autosave |
| Eyedropper | `shell/eyedropper.rs` | Armed from a parameter; magnifier grid follows the cursor over the Viewer; click picks; Shift+scroll widens the sampled area; commits through the undo path |

## 5. Keyboard shortcuts (the shipped set)

Global gate: skipped while any text field holds focus.

| Chord | Action | Source |
|---|---|---|
| Space | Toggle play | `app_update.rs` |
| K / L / J | Pause / play / step back (shuttle speeds await the ring buffer) | `app_update.rs` |
| ← / → | Step one frame (pauses playback) | `app_update.rs` |
| Home / End | First / last preview frame | `app_update.rs` |
| B / N | Work-area in / out at playhead | `app_update.rs` |
| Ctrl+Z / Ctrl+Shift+Z | Undo / redo | `shortcuts.rs` |
| Ctrl+S | Save | `shortcuts.rs` |
| Ctrl+, | Settings | `shortcuts.rs` |
| Ctrl+Shift+P | Command palette | `shortcuts.rs` |
| Ctrl+C / Ctrl+V | Copy / paste selected lane keyframes (via clipboard events, UI-7) | `shortcuts.rs` |
| Shift+F3 | Toggle graph editor | `shortcuts.rs` |
| Delete / Backspace | Selected keyframes first, else selected layer | `shortcuts.rs` |
| `*` (text event, layout-independent) | Add marker at playhead (works during playback) | `shortcuts.rs` |
| Ctrl+D | Duplicate selected layer | `shortcuts.rs` |
| `=` / `-` / `\` | Timeline zoom in / out / fit (1.4×, clamp 1–400 %) | `shortcuts.rs` |
| `[` / `]` | Move selected layer's in/out to playhead | `shortcuts.rs` |
| Alt+`[` / Alt+`]` | Trim selected layer's in/out to playhead | `shortcuts.rs` |

`lumit-keymap` (chords, contexts, conflict detection) exists as a pure crate but
the UI does not consult it yet — the port models the same action-id vocabulary so
the remappable keymap lands once, later, in both worlds or the winner.

## 6. Menus (Windows in-window bar)

- **File:** New project · Open project… · Import footage… · Save · Export comp…
  (stamps Settings → Export default) · Export preset ▸ (each preset) · Export for
  sharing ▸ Discord 50 MB / Small 10 MB
- **Edit:** Undo · Redo (enabled from the journal's can_undo/can_redo)
- **Composition:** New composition · Add solid layer · Add text layer · Add camera
  layer · Add adjustment layer · Add sequence layer · Cut clip at playhead ·
  Delete clip at playhead · Add marker at playhead · Detect beats ▸ (sensitivity
  slider 0–100 + Detect) · Clear beat markers · Add mask ▸ Rectangle/Ellipse/Star ·
  Composition settings…
- **Window:** Command palette… · Reset workspace · Settings…

## 7. Theme system (ported verbatim)

`theme.rs` — the only hex-bearing module. To port exactly:

- `Theme` struct: five surfaces + `viewer_surround` (exactly neutral, never
  mode-mirrored), four text tones, two hairlines, accent + hover,
  success/warning/error, `cache_disk`, 4 curve colours, 6 `LayerColours`,
  fixed `ScopeColours::STANDARD`.
- Seven `ColorScheme`s: Dark, Dark blue, Light, Gruvbox dark, Gruvbox light,
  Catppuccin Mocha, Catppuccin Latte (hex tables carried over digit-for-digit).
- `ShapeTokens`: SHARP {4, 6, 0, 0, 1.0, 0.0, no shadow} and ROUND
  {8, 12, 14, 10, 12.0, 12.0, soft shadow 0/4/16 @ 0x30 black}.
- `with_accent`: hover shifts +0x12 per channel on dark schemes, −0x12 on light.
- `AnimationLevel`: All 120 ms, Minimal 50 ms, None 0 (and cancel in-flight).
- Type scale: heading 16, body 12, button 12, small 11, mono 12 — Inter Medium
  (`assets/fonts/Inter-Medium.otf`, OFL) first in the family.
- Widget states: idle borderless (`surface_3` fill), hover `surface_4` +
  `hairline_strong` edge, active `hairline_strong` fill + accent edge; selection
  = accent at 50 % gamma; thin solid 6 px scrollbars; float shadow 0/15 blur 50
  @ 0x80 black; menu margin 12; item spacing 6×4; button padding 8×3.
- `label_colour(i)`: 8 layer-label chips drawn from the theme's own roles.
- `document_colour`: the one constructor for document-owned colours.

## 8. Icons

`icons.rs` — Iconoir (MIT) via an embedded icon font; 44 variants, one drawn by
hand (MotionBlur — ring plus five speed streaks on a 24×24 grid, from the
owner's artwork). Port carries the same variant→Iconoir-name table (e.g.
Pointer→cursor-pointer, Film→movie, GraphCurve→ease-curve-control-points,
Stopwatch→timer, KeyframeFilled→keyframe filled style…) and the same rules:
icons take the text colour of their state, emoji are banned, drawn icons are
painted not typeset.

## 9. Persisted state (what a workspace save carries)

`Shell` serde fields — the port must read/write an equivalent:

- dock tree (layout + shares + active tabs) and `floating` panels
- `color_scheme` (with legacy mode×variant migration, K-097), `accent_override`,
  `theme_shape`, `animation_level`
- `PerformanceSettings { ram_budget_mb (default half of RAM ≥ 2048), disk_cache_mb
  (51200), vram_cache_mb (512), background_fill (true), cache_root (None) }`
- `AutosaveSettings { interval_mins (5), keep (3 — from AUTOSAVE_KEEP) }`
- `InterfaceSettings { ui_scale (1.0), show_tooltips (true) }`
- `ExportSettings { default_preset (Custom), filename_template (None) }`

Plus per-project session restore (open comp tabs, fronted comp, playhead,
selection, twirl states) keyed by project path (OD-4).

## 10. Known parity traps

- The Viewer surround and scope colours must never follow the theme —
  grading-accuracy rules (15-DESIGN §2.1/§8) are easy to lose in a Material
  re-theme.
- UI scale applies on slider **release**, not per drag frame (K-117) — mid-drag
  re-scaling re-lays-out the slider under the cursor.
- "Show tooltips" off must kill every tooltip app-wide.
- Sharp shape must reproduce the pre-K-092 look byte-for-byte; Round is the
  card system. Both shapes share colours.
- The settings window opens on Appearance every time; fixed size on every page.
- Autosave defaults must match the engine constants, not literals.
- Completion notices are quiet; only genuine errors take the error tint.
