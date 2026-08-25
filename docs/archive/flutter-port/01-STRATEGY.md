# 01 — Strategy for the Flutter frontend alternative

## In plain terms

Lumit's interface is currently drawn by egui, a Rust library that repaints the
whole window every frame (an "immediate-mode" UI). This branch rebuilds the same
interface in Flutter, Google's UI toolkit, which describes the interface as a
tree of widgets and repaints only what changed ("retained-mode"). The engine —
everything that decodes, composites, caches and exports — stays exactly where it
is, in the Rust crates. Only the chrome changes hands.

## Why an alternative, not a replacement

The owner wants to evaluate Flutter's interface quality (text rendering, motion,
platform polish, ecosystem of widgets) against egui's. Until the Flutter frontend
reaches one-for-one parity **and** wins that evaluation, the egui frontend remains
the shipping one on `main`. This work lives on `flutter-frontend-alternative` and
must never destabilise the Rust workspace: the engine crates compile and test
unchanged with or without the Flutter tree present.

## Ground rules (binding for work on this branch)

1. **Parity before opinion.** The first pass reproduces today's behaviour —
   including behaviour the owner already knows they want to change. Known
   rough edges are logged in 05-PARITY-CHECKLIST under "post-parity fixes",
   not fixed in place. A truthful baseline is the whole point.
2. **The docs stay canonical.** Where 07-UI-SPEC and the shipped egui code
   disagree, the code is what parity means here (the port copies what *works*),
   but the disagreement gets a line in the checklist so the spec set can be
   reconciled later.
3. **The glossary binds Dart too.** Identifiers, UI strings, comments and
   commit messages in `flutter_ui/` follow docs/01-GLOSSARY.md — *layer* not
   track, *speed* not velocity, *Retime* not time remap, *export* not render
   (user-facing). British English, sentence case, no exclamation marks, no
   emoji, no punishment UI.
4. **No hex outside the theme.** `flutter_ui/lib/theme/theme.dart` is the one
   Dart file where colour literals may appear, mirroring the Rust rule
   (docs/15-DESIGN.md). Everything else reads the theme object. The existing
   CI grep is extended to the Dart tree when the port joins CI.
5. **Tests land with features.** Widget tests and plain Dart unit tests are the
   Flutter equivalents of the Rust rule (K-007): the theme tables, dock-tree
   logic, settings persistence and shortcut routing all carry tests from day one.
6. **Engine crates never depend on the UI** (docs/05-ARCHITECTURE.md) — that rule
   is unchanged; the bridge crate depends on engine crates, never the reverse.

## The phase plan

Phases are cumulative; each leaves the branch in a working, demonstrable state.

- **Phase F0 — scaffold and chrome (this session).** The Flutter project;
  the full theme port (all seven colour schemes, Sharp/Round shape tokens,
  accent override, animation levels); the Iconoir icon mapping; the settings
  model and Settings window (all five pages); the dock layout with the default
  workspace, resizable splits and tab pills; every panel present as a themed
  stub carrying its real name and chrome; the menu bar; the status line;
  the command palette; keyboard shortcut routing. No engine — a pure-Dart
  stand-in state object backs the controls.
- **Phase F1 — the bridge.** `flutter_rust_bridge` binding a new `lumit-bridge`
  crate; project open/save, the document model read path, ops dispatch
  (undo/redo journal intact); the Project panel live.
- **Phase F2 — the Viewer.** wgpu renders into a shared texture
  (D3D11 interop on Windows) presented through Flutter's `Texture` widget;
  transport, playback and the missing-footage slate live.
- **Phase F3 — the Timeline.** Rows, columns, switches, clip bars, keyframe
  lanes, the graph lens, drag interactions.
- **Phase F4 — the editors.** Effect controls, effects & presets, scopes,
  hierarchy, eyedropper, export dialogue and queue.
- **Phase F5 — evaluation.** Side-by-side against egui on the owner's machine;
  the decision whether Flutter becomes the frontend.

## Verification

- `flutter analyze` clean (no infos suppressed) and `flutter test` green are
  the branch's local gate; a CI job is added when the branch first opens a PR.
- Phase F0 is judged by eye against the running egui build: same default
  workspace, same settings pages, same theme swatches. Screenshots of both go
  in the PR description.
- From F1 on, behaviour parity is judged against the checklist, surface by
  surface.

## What is deliberately out of scope for the port

- macOS native menu bar (`native_menu.rs`, muda) — Flutter's
  `PlatformMenuBar` covers it when a macOS pass happens; Windows keeps the
  in-window bar either way.
- The remappable keymap (`lumit-keymap`) is modelled but its Settings page,
  like in the egui frontend, does not exist yet — parity means porting the
  *shipped* shortcut set.
- Anything the audit (docs/implementation-audit-2026-07-20.md) marks
  unimplemented in the egui frontend: parity with what works, not with the spec's
  aspirations.
