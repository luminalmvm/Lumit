# 04 — egui concept → Flutter equivalent

One row per mechanism the egui frontend leans on. "Custom" means a widget we
write and own in `flutter_ui/lib/` — the port prefers a small owned widget over
bending a Material one when the house look is at stake.

| egui mechanism | Where used | Flutter equivalent |
|---|---|---|
| `Theme::apply` on `egui::Context` | everywhere | `LumitTheme` object handed down by an `InheritedWidget` (`LumitThemeScope`); **not** Material's `ThemeData` (Material widgets are avoided in chrome — see below) |
| `egui_tiles::Tree<Panel>` | dock | Custom dock: a serialisable tree of `Split`(axis, weighted children)/`Tabs`(children, active)/`Pane`(panel) nodes, rendered with `Row`/`Column` + `Expanded(flex)` and custom drag dividers. Tab drag/re-dock arrives in a later slice (checklist) |
| `egui::Modal` (Settings, palette) | dialogs | `showDialog` with a custom themed dialog shell (dimmed barrier click closes, Escape closes) |
| `ui.menu_button` bar | menu bar | Custom `MenuBarWidget` (hover-open, themed) — Material's `MenuAnchor` used underneath where it does not fight the look |
| `egui::DragValue` | settings, inspector | Custom `DragValueField`: horizontal-drag adjusts, click to type, range clamp, speed |
| `egui::Slider` | UI scale, sensitivity | Custom thin slider on theme colours (commit-on-release variant for UI scale, K-117) |
| `ComboBox`/`bare_dropdown` | scheme, shape, presets | Custom `BareDropdown` (label + caret, floating themed popup list) |
| `SelectableLabel` sidebar | settings pages | Custom `SidebarEntry` |
| Checkbox | settings, switches | Custom 14 px themed checkbox |
| `color_edit_button_srgb` | accent, solids | Swatch button opening a small custom HSV/RGB picker popup |
| `egui::TextEdit` | search, template, values | `EditableText`/`TextField` stripped of Material decoration, themed selection (accent at 50 %) |
| `Painter` (rects, lines, glyphs) | timeline, scopes, graph | `CustomPainter` per surface (timeline lanes, ruler, scope traces, graph curves, motion-blur icon) |
| `TextureHandle` preview | Viewer | `Texture` widget (F2) / `RawImage` fallback |
| `ctx.input` key handling | shortcuts | A root `Focus` + `Shortcuts`/`Actions` map, with the same "skip while a text field is focused" gate (`FocusManager` primary focus check) |
| Hover tooltips + global disable | everywhere | Custom `LumitTooltip` reading `InterfaceSettings.showTooltips` (Flutter's `Tooltip` cannot be globally disabled) |
| `ctx.set_pixels_per_point` | UI scale | `FractionalTranslation`-free: wrap the app in `MediaQuery` override / `transform: Matrix4.diagonal3Values(scale…)` via `Transform.scale` on the root, DPI-aware |
| `Shadow` popup/window | menus, dialogs | `BoxShadow` with the same offset/blur/alpha numbers |
| eframe storage | workspace persistence | JSON file in the config dir (03-ARCHITECTURE §Persistence) |
| `egui::Context::request_repaint` cadence | playback, scopes | Streams drive `notifyListeners`; `Ticker` only where a steady cadence is genuinely needed |
| Right-click `context_menu` | panes, rows, comp strip | Custom themed context menu on `Listener` secondary-tap |
| Pop-out OS windows | floating panels | Deferred: Flutter multi-window on desktop is still maturing; the checklist carries it, and the dock keeps panels reachable meanwhile |

## Why not Material chrome

Material widgets bring their own metrics, splash effects and motion, all of
which fight the house design (12 px body text, 16 px interact height,
borderless idle widgets, no ripple). The port uses `WidgetsApp`-level
infrastructure (focus, overlay, navigation) with owned leaf widgets, so every
colour and metric comes from the ported theme tokens — the same discipline the
Rust side enforces with the no-hex lint.

## Animation levels

`AnimationLevel` maps to a house `motionDuration(theme)` helper: All = 120 ms,
Minimal = 50 ms, None = `Duration.zero` — every implicit animation in owned
widgets reads it, which is *better* coverage than egui offered (its menus never
animated; ours simply follow the setting).
