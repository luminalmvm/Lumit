# Multi-window — impl note (binding for its topic; researched 2026-08-23)

Feeds docs/07-UI-SPEC.md (panels, workspaces, dialogs) and docs/17-BRIDGE-CONTRACT.md
(the Viewer texture transport). The specs say *what*; this note pins what Flutter's
multi-window support actually is today, what it would cost to adopt, and what to
test before believing any of it. Every claim here was checked against flutter.dev
release notes, the flutter/flutter source on the main channel, and the tracking
issues on 2026-08-23 — this is a moving target, so re-verify the §1 status line
before acting on this note.

## In plain terms

Today Lumit is one operating-system window: every panel, dialog and the Viewer
live inside a single frame that Flutter draws. "Multi-window" means the app can
open real, separate OS windows — a floating export dialog the taskbar knows
about, a second Viewer on a second monitor, a torn-off timeline. Flutter has
been building exactly this for desktop since 2019, in partnership with Canonical,
and the design is the right one for us: **one** Flutter engine and **one** Dart
program drive **many** windows, so every window sees the same application state —
no copies, no inter-process plumbing. Each window is just another branch of the
same widget tree.

The catch is simple and decisive: **as of August 2026 none of it has shipped in
a stable Flutter release.** It exists only on Flutter's *main* (development)
channel, behind an opt-in flag, and the API is stamped "internal — we will break
this even in patch versions". Lumit pins stable Flutter (currently 3.47.1). So
this note is a map for later, plus one cheap spike worth doing early: proving
the engine's zero-copy Viewer texture can appear in a second window at all,
because that is the one Lumit-specific risk no release note will ever answer.

## 1. Status: current vs required Flutter (the hard blocker)

- **Installed / pinned**: Flutter 3.47.1 stable (2026-08-19), Dart 3.13.1
  (re-checked 2026-08-25 on the 3.47 upgrade).
  `flutter_ui/pubspec.yaml` asks for Dart `>=3.6.0 <4.0.0` and pins
  `flutter_rust_bridge: 2.12.0` exactly (the codegen and runtime versions must
  match — see §7).
- **Multi-window ships in**: still no stable release. Flutter 3.44 (Google I/O,
  May 2026) says windowing is "only available on the main channel and not yet
  intended for production use", and 3.47 stable (August 2026) only calls it
  "experimental multi-window progress". This is not a matter of an unset flag:
  in the installed 3.47.1, `windowingFeature` in
  `packages/flutter_tools/lib/src/features.dart` declares
  `master: FeatureChannelSetting(available: true)` and **no stable setting at
  all**, so `flutter config --enable-windowing` cannot turn it on here. The
  `examples/multiple_windows` reference app still requires the **main channel**
  plus that flag.
- **API stability**: everything in §2 lives in
  `packages/flutter/lib/src/widgets/_window.dart` and is annotated `@internal`
  with the doc warning "Do not use this API in production applications or
  packages published to pub.dev. Flutter will make breaking changes to this API,
  even in patch versions." They mean it: across the 3.47 cycle `preferredSize`
  became `size` and the `decorated` flag went away — neither name survives in
  3.47.1. (`RegularWindow` does still exist there, alongside `DialogWindow`,
  `TooltipWindow`, `PopupWindow` and `SatelliteWindow`; the rename to `Window`
  this note predicted on 2026-08-23 has not landed.)
- **What an upgrade drags in**: the main channel means a pre-release Dart SDK
  (fine for our `<4.0.0` constraint) and re-running frb codegen plus a
  `lumit_bridge` dylib rebuild. flutter_rust_bridge tracks the Dart SDK, not the
  Flutter channel — latest is 2.13.0 (Aug 2026) and 2.12.0 already runs on
  3.44 — so frb is a chore, not a blocker. CI is the real cost: our suite runs
  on stable; a main-channel spike cannot gate merges and must live on a branch.

**Consequence**: Lumit MUST NOT take a production dependency on the windowing
API until it reaches the stable channel un-flagged. Until then, everything
multi-window is spike work.

## 2. The API as it actually is (main channel, 2026-08-23)

One engine, one root isolate, many `FlutterView`s — one per window. Windows
share the widget/element tree and all Dart state; each has its own render,
layer, focus and semantics trees. The embedder (Win32 / Cocoa / GTK) creates
the native windows itself, driven from Dart; the runner no longer hand-builds
them.

Window archetypes, each a controller class plus a widget that hosts the
window's content subtree:

| Archetype | Controller | Widget | Notes |
|---|---|---|---|
| Regular | `WindowController` (also `.shrinkWrap` sized-to-content) | `Window(controller:, child:)` | Normal resizable, taskbar-visible window |
| Dialog | `DialogWindowController` (takes `parent:` for modality) | `DialogWindow` | Modal to its parent when parented; `.shrinkWrap` variant too |
| Satellite | `SatelliteWindowController` (parent + `WindowPositioner`) | `SatelliteWindow` | Auxiliary window that keeps position relative to its parent — the tear-off-panel shape |
| Popup | `PopupWindowController` (parent + `anchorRect` + positioner) | `PopupWindow` | Transient: menus, context menus |
| Tooltip | `TooltipWindowController` (parent + anchor) | `TooltipWindow` | Anchored info bubbles |

Around them:

- `BaseWindowController` — sealed base; one root `FlutterView` per controller;
  exposes `contentSize`.
- Controller constructors take `size`, `constraints`, `title`, and a `delegate`
  mixin with `onWindowCloseRequested()` / `onWindowDestroyed()`.
- `WindowManager(initialWindows: [WindowEntry(controller:, builder:)])` — the
  root widget that renders the set of windows; register/unregister at runtime
  via `WindowRegistry`.
- `WindowScope` — an `InheritedModel` any widget can read: `contentSizeOf`,
  `titleOf`, `isActivatedOf`, `isMinimizedOf`, `isMaximizedOf`,
  `isFullscreenOf`, `isDestroyedOf`.
- `createDefaultWindowingOwner()` — platform factory; returns a throwing stub
  when the feature flag is off.
- Platform escape hatch: platform-specific controller subclasses expose
  `windowHandle` — a real `HWND` / `NSWindow` / `GtkWindow` (landed in the 3.47
  cycle).
- Material integration: with windowing enabled, `showDialog` creates a real
  child dialog **window** on supporting platforms — existing dialog code gets
  OS windows without rewrites (3.44 cycle).

Platform maturity (3.47 cycle): Windows and Linux are furthest along (regular,
dialog, popup, sized-to-content all landed); macOS trails (popups landed
earlier, but e.g. flutter/flutter#184701 — windows show before Flutter content
is ready). Known Windows quirks are being fixed on main as they surface
(#187436 taskbar-activation focus, #188016 z-order).

## 3. Drag-and-drop between windows: not provided

The windowing API gives no cross-window drag primitive, and the design doc does
not address it. `Draggable`/`DragTarget` ride a single view's pointer-event
stream: a drag that starts in window A never delivers to a `DragTarget` in
window B, same engine or not. The options, in Lumit-preference order:

1. **Shared state instead of a drag.** Because every window is the same isolate,
   "drag between windows" can be an armed action: press in window A records the
   payload in Dart state (ultimately in the engine, per the thin-view rule);
   window B's targets highlight and accept on click. No OS drag at all.
2. **A real OS drag** (payload leaves via the platform's drag-and-drop —
   `DoDragDrop` on Windows), using a package such as `super_drag_and_drop`.
   This also covers dragging footage in from Explorer, which we want anyway,
   and works between our own windows because the drop side is just an OS drop.
3. Do nothing: keep any panel pair that needs dragging between them dockable in
   the same window. Tear-off is a convenience, not the only home of a panel.

Option 1 is the default; option 2 only if a real spring-loaded drag feel is
demanded. Do not build cross-window `Draggable` glue — it fights the framework.

## 4. The Viewer texture across windows (the Lumit-specific risk)

How the zero-copy path hangs together today (17-BRIDGE-CONTRACT
§transport): the engine renders into a shared D3D12 texture; Dart passes the
handle over the `lumit/viewer_texture` channel; the runner's
`ViewerTextureBridge` (flutter_ui/windows/runner/viewer_texture_bridge.cpp)
wraps it in a `FlutterDesktopGpuSurfaceDescriptor` and registers it with the
**engine's** texture registrar; the `Texture(textureId)` widget samples it.

What multi-window changes, and does not:

- **Registration is engine-scoped, not window-scoped.** The bridge is built
  from a `FlutterDesktopPluginRegistrarRef` and its `texture_registrar()`;
  there is one engine, so a registered `textureId` is meaningful in every
  window's subtree. Nothing about registration needs to move.
- **Compositing is the unknown.** Each window has its own swapchain and its
  Scene is composited per view by new embedder code
  (`flutter::WindowManager` in the Windows embedder). Whether the external
  GPU-surface texture layer type is handled in secondary windows' compositors
  is documented nowhere and has no tracking issue we could find, in either
  direction. Assume nothing.
- **Frame fan-out is the second unknown.** `MarkTextureFrameAvailable`
  schedules a repaint; with the same texture visible in two windows, both
  views must repaint per frame-ready. Untested territory.
- **The runner changes shape.** Today the runner creates the one window and
  hooks the bridge in `FlutterWindow::OnCreate`. Under windowing the framework
  creates windows and the runner shrinks to engine setup; the bridge survives
  because it hangs off the plugin registrar, but the hook point moves. Part of
  the spike, not a redesign.

**Test first, before any UI planning**: the §6 spike, step 2. If a second
window cannot composite the engine texture, "Viewer on a second monitor" is
blocked upstream and only dialogs/panels-without-Viewer are on the table until
the embedder gains it.

## 5. Mapping Lumit's dialogs onto the archetypes

Everything below is an in-app overlay today (flutter_ui/lib/shell/): export
dialog, composition settings, project settings, settings window, first-run
(welcome), about, update, recovery, precompose, theme editor and name dialog,
command palette, fx console. The mapping, when the API is stable:

- **DialogWindow (parented to the main window)**: export dialog, composition
  settings, project settings, precompose, recovery, update, about, theme name.
  These are modal question-and-answer surfaces — exactly the archetype. Most
  can ride the `showDialog`-becomes-a-window integration with no per-dialog
  work, which is the strongest reason to keep writing dialogs as ordinary
  `showDialog` overlays now.
- **Regular `Window`**: settings window, theme editor, first-run/welcome, and a
  future render-queue window — long-lived, non-modal, useful on the taskbar.
  The welcome window is the natural *first* adoption target: it exists before
  a project is open and contains no Viewer.
- **SatelliteWindow**: torn-off panels (a second Viewer, a floating graph
  editor). Gated on §4.
- **PopupWindow**: menu bar menus and the command palette, eventually — lowest
  priority; the in-window versions work.

None of this changes what a dialog *is* in Dart: a widget subtree. The move is
re-parenting subtrees, not rewriting them — which is why no preparatory
abstraction layer is needed now.

## 6. Migration order

1. **Now: nothing in main.** Stay single-window on stable. Keep dialogs as
   `showDialog`/overlay widgets (that is the compatibility path, §5).
2. **Cheap spike (any time, throwaway branch, local only)**: main-channel
   Flutter + `flutter config --enable-windowing`; wrap the app in
   `WindowManager`; open one extra regular window; then put
   `Texture(textureId)` for the already-registered Viewer texture in it —
   first alone, then simultaneously with the main Viewer. Screenshot both
   (real-window technique). This answers §4 with a day's work and tells us
   whether to ever promise multi-monitor Viewers.
3. **When windowing reaches stable un-flagged** (re-check §1): upgrade Flutter,
   re-run frb codegen, rebuild the dylib, adopt the `WindowManager` root with
   the main window only. Zero visible change; lands the runner rework.
4. Move the welcome window, then the dialogs (mostly free via `showDialog`),
   then the settings/theme/render-queue regular windows.
5. Satellite tear-off panels, gated on the §4 spike result; §3 option 1 for
   cross-window interactions.
6. Popup menus last, if ever.

## 7. Traps checklist

- The API names in this note **will** be stale; the 3.47 cycle alone renamed
  the flagship widget. Re-read `_window.dart` on main before writing code.
- `--enable-windowing` breaks plain `flutter test`: `WindowingOwner[Platform]
  must be created after the engine has been initialized` (flutter/flutter
  #178706). Widget tests cannot exercise any of this — expected; the embedder
  is out of their reach (see the repo's real-window testing rule).
- frb is pinned exactly (2.12.0): a Flutter/Dart bump means codegen re-run
  **and** a `lumit_bridge` dylib rebuild, or every frb test fails with
  "found 0 widgets".
- One isolate, many windows: a panicking dialog window is the whole app, same
  as today — the never-crash rule does not relax because a window is small.
- macOS trails Windows/Linux; do not let a Windows-first spike promise macOS
  parity dates.
- Modality: a `DialogWindowController` without `parent:` is modeless; Lumit's
  export dialog must pass the main controller or it stops blocking edits.
- Do not build cross-window `Draggable` plumbing (§3).

## 8. Test plan (with the feature, under repo constraints)

Widget tests cannot reach the embedder, so everything windowing-shaped is an
integration test on a real window (integration_test + PrintWindow screenshots,
per the established technique), Windows-first, and — until the API is stable —
run manually on the spike branch, not in CI.

- **Second window opens and paints**: open a regular window, screenshot it,
  assert non-blank and expected background colour (catches the macOS
  blank-before-content class of bug on Windows too).
- **Texture in a second window**: register the Viewer texture, host it in the
  second window, engine renders a known pattern, screenshot-assert pixels —
  the §4 question, same shape as `integration_test/shared_texture_test.dart`.
- **Two viewers, one texture**: both windows showing it, advance a frame,
  assert both screenshots changed (frame fan-out).
- **Modality**: parented dialog open → clicks in the parent do nothing (assert
  no state change), dialog closes via its own button.
- **Close-request delegate**: `onWindowCloseRequested` fires and can veto
  (unsaved-changes shape).
- **Single-window regression**: the full existing suite still passes with the
  windowing root in place but only one window — step 3 of §6 must be invisible.

## Sources (checked 2026-08-23)

- What's new in Flutter 3.44 / 3.47 — flutter.dev/blog (windowing "only
  available on the main channel", "not yet intended for production";
  `showDialog` child windows; popups; `windowHandle`; sized-to-content).
- flutter/flutter `examples/multiple_windows` (main channel + `--enable-windowing`).
- flutter/flutter `packages/flutter/lib/src/widgets/_window.dart` on master
  (the §2 API surface and the `@internal` breaking-changes warning).
- Desktop Multi-Window Support design doc (single engine, multiple views, one
  isolate; per-window Scenes; non-goals).
- Issues/PRs: #30701 (original tracker), #173824 (channel status), #178706
  (test breakage), #184701 (macOS blank windows), #187436/#188016 (Windows
  focus/z-order), #181861 (`showDialog` windowing), #173715 (Win32 regular
  windows), #184516/#185866 (popups), #186829 (sized-to-content Win32),
  #184662 (`windowHandle`).

## Feeds

07 (dialogs, panels, workspaces), 17 (Viewer texture transport), 16 (any
roadmap line that promises multi-monitor or torn-off panels).
